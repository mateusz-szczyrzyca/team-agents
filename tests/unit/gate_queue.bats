#!/usr/bin/env bats
# Waiting for a lock was a lottery: every waiter retried flock every 15 s, and whoever tried when the holder let go
# took it — a human waiting since 09:48 was passed by an agent come at 09:54. Both locks — the harness's gate lock
# (agent gate) and this repository's test lock (tools/test-lock.sh) — now keep a line: a waiter puts its place in a
# directory beside the lock (<lock>.queue, a file <start in ns>-<pid> each) and takes the lock only first in line; a
# run that does not wait refuses while someone waits, even with the lock free; a dead waiter's place does not hold
# the line. And agents ran make gate directly, from checkpoints older than agent gate: the run raced nobody, but its
# result was not recorded. In a task's worktree, the test lock's gate now runs through agent gate; agent gate marks
# its own run (AGENT_GATE_RUN), so the repository's gate inside it runs as before.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# lock_env — a test lock of the test's own: the gate running these tests holds the real one, and may run inside agent
# gate
lock_env() {
  unset TEST_LOCK_HELD TEST_LOCK_WAIT GATE_WAIT AGENT_GATE_RUN
  export TEST_LOCK_FILE="$BATS_TEST_TMPDIR/test.lck"
  LOCK="$REPO_ROOT/tools/test-lock.sh"
}

# waiting_before queue — a live process with a place in line in queue, older than any new one; sets OLDER (its pid)
waiting_before() {
  sleep 60 &
  OLDER=$!
  mkdir -p "$1"
  : > "$1/0000000000000000001-$OLDER"
}

# not_started_in seconds file — the file did not appear within that many seconds
not_started_in() { sleep "$1"; [ ! -e "$2" ]; }

# appears file — the file appears within 10 s
appears() { for _ in $(seq 1 100); do [ -e "$1" ] && return 0; sleep 0.1; done; return 1; }

@test "test lock: a waiter goes after the one in line before it, even with the lock free" {
  lock_env
  waiting_before "$TEST_LOCK_FILE.queue"
  TEST_LOCK_WAIT=1 "$LOCK" mine touch "$BATS_TEST_TMPDIR/ran" &
  not_started_in 2 "$BATS_TEST_TMPDIR/ran"
  kill "$OLDER"
  appears "$BATS_TEST_TMPDIR/ran"
  wait
  [ -z "$(ls -A "$TEST_LOCK_FILE.queue")" ]   # its place is gone after it
}

@test "test lock: a run that does not wait refuses while someone waits in line, even with the lock free" {
  lock_env
  waiting_before "$TEST_LOCK_FILE.queue"
  run "$LOCK" mine true
  [ "$status" -eq 75 ]
  [[ $output == *"another tests are running now - please wait before you try again"* ]]
  [[ $output == *"1 waiting in line"* ]]
  kill "$OLDER"
}

@test "test lock: a dead waiter's place does not hold the line" {
  lock_env
  sleep 0 &
  dead=$!
  wait "$dead"
  mkdir -p "$TEST_LOCK_FILE.queue"
  : > "$TEST_LOCK_FILE.queue/0000000000000000001-$dead"
  run "$LOCK" mine true
  [ "$status" -eq 0 ]
  [ ! -e "$TEST_LOCK_FILE.queue/0000000000000000001-$dead" ]
}

# stub_agent task — an agent command on PATH that names task for every directory (none: empty) and records a gate
stub_agent() {
  mkdir -p "$BATS_TEST_TMPDIR/bin"
  cat > "$BATS_TEST_TMPDIR/bin/agent" << EOF
#!/usr/bin/env bash
case \$1 in
  _task_of_dir) printf '%s\n' "$1" ;;
  gate) shift; echo "\$*" > "$BATS_TEST_TMPDIR/agent-gate" ;;
esac
EOF
  chmod +x "$BATS_TEST_TMPDIR/bin/agent"
  PATH="$BATS_TEST_TMPDIR/bin:$PATH"
}

@test "test lock: the gate in a task's worktree runs through agent gate, so its result is recorded" {
  lock_env
  stub_agent 20261110-1-solo
  run "$LOCK" gate touch "$BATS_TEST_TMPDIR/direct"
  [ "$status" -eq 0 ]
  [ "$(cat "$BATS_TEST_TMPDIR/agent-gate")" = "20261110-1-solo" ]
  [ ! -e "$BATS_TEST_TMPDIR/direct" ]
  GATE_WAIT=1 "$LOCK" gate touch "$BATS_TEST_TMPDIR/direct"   # waiting: agent gate waits in line
  [ "$(cat "$BATS_TEST_TMPDIR/agent-gate")" = "20261110-1-solo --wait" ]
}

@test "test lock: inside agent gate, outside a task's worktree, or for another target, it runs as before" {
  lock_env
  stub_agent 20261110-1-solo
  AGENT_GATE_RUN=20261110-1-solo "$LOCK" gate touch "$BATS_TEST_TMPDIR/inside"
  [ -e "$BATS_TEST_TMPDIR/inside" ]
  "$LOCK" unit-bats touch "$BATS_TEST_TMPDIR/other"
  [ -e "$BATS_TEST_TMPDIR/other" ]
  stub_agent ""
  "$LOCK" gate touch "$BATS_TEST_TMPDIR/outside"
  [ -e "$BATS_TEST_TMPDIR/outside" ]
  [ ! -e "$BATS_TEST_TMPDIR/agent-gate" ]
}

# with_task gate — a solo task waiting (on its questions) in its worktree, the gate's command set; sets t and wt
with_task() {
  printf "AGENT_REVIEW_GATE='%s'\n" "$1" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
  wt=$(meta "$t" WORKTREE)
}

@test "agent gate: --wait goes after the one in line before it; without --wait it refuses while one waits" {
  with_task 'true'
  waiting_before "$AGENTS_HOME/gate.queue"
  run agent gate "$t"
  [ "$status" -eq 75 ]
  [[ $output == *"1 waiting in line"* ]]
  agent gate "$t" --wait > /dev/null &
  gate=$!
  sleep 2
  kill -0 "$gate"   # still in line
  kill "$OLDER"
  wait "$gate"
  [ -z "$(ls -A "$AGENTS_HOME/gate.queue")" ]
  agent stop --running > /dev/null
}

@test "agent gate runs the repository's gate with AGENT_GATE_RUN, its task: the gate knows it runs inside agent gate" {
  with_task "printenv AGENT_GATE_RUN > $BATS_TEST_TMPDIR/run"
  agent gate "$t" > /dev/null
  [ "$(cat "$BATS_TEST_TMPDIR/run")" = "$t" ]
  agent stop --running > /dev/null
}

@test "agent _task_of_dir names the task whose worktree a directory is in, and nothing elsewhere" {
  with_task 'true'
  [ "$(agent _task_of_dir "$wt")" = "$t" ]
  run agent _task_of_dir /
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  agent stop --running > /dev/null
}

@test "the precheck runs the repository's gate with AGENT_GATE_RUN too: under its own gate lock, never sent to agent gate" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf "AGENT_REVIEW_GATE='printenv AGENT_GATE_RUN >> %s; exit 1'\n" "$BATS_TEST_TMPDIR/run" >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-ready"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep "gate" "$HOME/notifications.log" 150
  grep -q -- "-lead$" "$BATS_TEST_TMPDIR/run"
  agent stop --running > /dev/null
}

@test "the repository's own checkout is no task's worktree, even where a task (an investigation team's) names it" {
  with_task 'true'
  root=$(meta "$t" REPO_ROOT)
  inv=20261110-000001-investigation-team
  mkdir -p "$AGENTS_HOME/tasks/$inv"
  printf 'ID=%s\nKIND=investigation_team\nREPO_ROOT=%s\nWORKTREE=%s\n' "$inv" "$root" "$root" > "$AGENTS_HOME/tasks/$inv/meta.env"
  run agent _task_of_dir "$root"
  [ "$status" -eq 0 ]
  [ -z "$output" ]   # a human's make gate there runs as before, not through agent gate
  [ "$(agent _task_of_dir "$wt")" = "$t" ]
  agent stop --running > /dev/null
}
