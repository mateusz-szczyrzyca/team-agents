#!/usr/bin/env bash
# tools/test-lock.sh label command [args...] — runs a test command when no other test run on this machine holds the
# test lock, and holds it meanwhile. Every Makefile target that runs tests goes through it: agents ran make gate at the
# same time in their worktrees, and their runs raced — one passed, another failed on the other's load or files.
#
# - The lock: TEST_LOCK_FILE (/tmp/.make.command.lck), held with flock; the holder is written into it (label,
#   directory, pid, process group, start). The file stays: removing it while another run waits on it would let two
#   runs hold "the" lock at once.
# - Busy: refuses at once with the message every agent knows, and who holds it; exit 75. TEST_LOCK_WAIT=1 (or
#   GATE_WAIT=1) waits instead.
# - A holder that died frees the lock at once: the kernel releases a flock with the last process holding it.
# - A holder alive past TEST_LOCK_STALE_MIN (10) minutes is taken for hung: its test run (process group) is stopped
#   and the lock taken over. A test run takes some 5 minutes.
# - A run inside a run (make gate's steps, the tests of the gate) does not lock again: TEST_LOCK_HELD is set.
# - Waiters keep a line, so the lock goes in turn, not to whoever retries when it is let go: a waiter's place is a file
#   in TEST_LOCK_FILE.queue (<start in ns>-<pid>), the lock goes only to the first in line, and a run that does not wait
#   refuses while someone waits, even with the lock free. A dead waiter's place is removed.
# - The gate in an agent's worktree runs through agent gate, which records its result (agent gate-status): an agent's
#   make gate, from habit, is otherwise a run nobody can check. agent gate marks its own run (AGENT_GATE_RUN): the gate
#   inside it runs here as before.
set -uo pipefail

LOCK_FILE=${TEST_LOCK_FILE:-/tmp/.make.command.lck}   # the machine's test lock
STALE_MIN=${TEST_LOCK_STALE_MIN:-10}                  # a holder older than this many minutes is taken for hung
WAIT=${TEST_LOCK_WAIT:-${GATE_WAIT:-}}                # non-empty: wait for the lock instead of refusing
EXIT_BUSY=75                                          # EX_TEMPFAIL: try again later
QUEUE=$LOCK_FILE.queue                                # the waiters' line: a file each, <start in ns>-<pid>
MESSAGE="another tests are running now - please wait before you try again"

if (($# < 2)); then
  echo "usage: tools/test-lock.sh label command [args...]" >&2
  exit 2
fi
label=$1
shift

if [[ -n ${TEST_LOCK_HELD:-} ]]; then   # a run inside a run that holds the lock
  exec "$@"
fi

# the gate in a task's worktree, not inside agent gate: through agent gate, which records its result
if [[ $label == gate && -z ${AGENT_GATE_RUN:-} ]] && command -v agent > /dev/null; then
  task=$(agent _task_of_dir "$PWD" 2> /dev/null) || task=""
  if [[ -n $task ]]; then
    echo "test-lock: in the worktree of task $task the gate runs through agent gate, which records its result" >&2
    exec agent gate "$task" ${WAIT:+--wait}
  fi
fi

# waiters — the live waiters' places in line, oldest first; a dead waiter's place is removed
waiters() {
  local f pid
  for f in "$QUEUE"/*; do
    [[ -e $f ]] || continue
    pid=${f##*-}
    if [[ $pid =~ ^[0-9]+$ ]] && kill -0 "$pid" 2> /dev/null; then
      printf '%s\n' "${f##*/}"
    else
      rm -f "$f"
    fi
  done
}

# holder_field name — a field of the holder written into the lock file
holder_field() { sed -n "s/^$1=//p" "$LOCK_FILE" 2> /dev/null | head -n 1; }

# holder — who holds the lock, in one line
holder() {
  printf '%s in %s (pid %s), for %s min' "$(holder_field label)" "$(holder_field dir)" "$(holder_field pid)" \
    "$(( ($(date +%s) - $(holder_start)) / 60 ))"
}

# holder_start — when the holder took the lock (epoch seconds); the file's time when it does not say
holder_start() {
  local s
  s=$(holder_field start)
  [[ $s =~ ^[0-9]+$ ]] || s=$(stat -c %Y "$LOCK_FILE" 2> /dev/null || date +%s)
  printf '%s' "$s"
}

if ! exec 9>> "$LOCK_FILE"; then
  echo "test-lock: cannot open $LOCK_FILE" >&2
  exit 1
fi

# take_over — stops a holder past TEST_LOCK_STALE_MIN and takes the lock (status 0); 1: the holder is not stale
take_over() {
  local pg
  (($(date +%s) - $(holder_start) >= STALE_MIN * 60)) || return 1
  echo "test-lock: $(holder) — past ${STALE_MIN} min, taken for hung: its test run is stopped" >&2
  pg=$(holder_field pgid)
  if [[ $pg =~ ^[0-9]+$ ]] && ((pg > 1)); then
    kill -TERM -- "-$pg" 2> /dev/null || echo "test-lock: its process group $pg is gone already" >&2
    flock -w 10 9 && return 0
    kill -KILL -- "-$pg" 2> /dev/null || echo "test-lock: its process group $pg is gone already" >&2
  fi
  flock -w 10 9 && return 0
  echo "test-lock: the hung test run did not let go of the lock; see: cat $LOCK_FILE" >&2
  exit 1
}

# acquire — takes the lock in turn: at once when free and nobody waits; busy or someone waiting: refuses (exit 75), or
# with TEST_LOCK_WAIT takes a place in line and waits for its turn; the first in line stops a stale holder
acquire() {
  local mine="" first n
  if [[ -n $WAIT ]]; then
    mkdir -p "$QUEUE" || { echo "test-lock: cannot create $QUEUE" >&2; exit 1; }
    mine="$(date +%s%N)-$$"
    : > "$QUEUE/$mine" || { echo "test-lock: cannot take a place in $QUEUE" >&2; exit 1; }
  fi
  while :; do
    first=$(waiters | head -n 1)
    if [[ -z $first || $first == "$mine" ]]; then
      flock -n 9 && break
      take_over && break
    fi
    if [[ -z $WAIT ]]; then
      n=$(waiters | wc -l)
      if ((n > 0)); then
        printf '%s\n  %s waiting in line for it\n' "$MESSAGE" "$n" >&2
      else
        printf '%s\n  holder: %s; taken for hung after %s min\n' "$MESSAGE" "$(holder)" "$STALE_MIN" >&2
      fi
      exit "$EXIT_BUSY"
    fi
    if [[ $first == "$mine" ]]; then
      flock -w 15 9 && break
    else
      sleep 1
    fi
  done
  [[ -z $mine ]] || rm -f "$QUEUE/$mine"
}

acquire
export TEST_LOCK_HELD=$$

# the command in its own process group: a hung run is stopped as a whole, the agent's shell around it is not. Job
# control leaves a background command the caller's stdin: from an interactive terminal, a test that read it would be
# stopped (SIGTTIN) and hang the run, so the tests read /dev/null
set -m
"$@" < /dev/null &
child=$!
set +m
{
  printf 'label=%s\ndir=%s\npid=%s\npgid=%s\nstart=%s\nstarted=%s\n' \
    "$label" "$PWD" "$$" "$child" "$(date +%s)" "$(date -Is)"
} > "$LOCK_FILE"

# stop — an interrupted test run stops its commands: nothing of it keeps the lock
stop() {
  kill -TERM -- "-$child" 2> /dev/null || echo "test-lock: the test run had ended" >&2
  wait "$child" 2> /dev/null
  exit 143
}
trap stop INT TERM HUP

wait "$child"
