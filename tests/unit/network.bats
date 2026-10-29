#!/usr/bin/env bats
# A run that ends because the network or the provider cannot be reached (a DNS timeout, a refused connection —
# an ISP's night works) is not the agent's failure: the task pauses for AGENT_NETWORK_PAUSE_MIN and goes on; only
# AGENT_NETWORK_MAX_PAUSES such pauses in a row fail it.

load helpers

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a network failure pauses the task for AGENT_NETWORK_PAUSE_MIN; its timer resumes it" {
  printf 'AGENT_NETWORK_PAUSE_MIN=15\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "network-down", "resume": "done"}}'
  before=$(date +%s)
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" paused
  [ "$(meta "$t" PAUSE_REASON)" = network ]
  wake=$(meta "$t" WAKE_AT)
  (( wake >= before + 15 * 60 + 60 && wake <= before + 15 * 60 + 60 + 30 ))
  wait_grep "unreachable" "$HOME/notifications.log"
  sed -i 's/^WAKE_AT=.*/WAKE_AT=1/' "$AGENTS_HOME/tasks/$t/meta.env"   # the timer fires after the pause
  agent _wake "$t"
  wait_status "$t" 'done'
}

@test "a network failing again and again fails the task after AGENT_NETWORK_MAX_PAUSES, with the reason" {
  printf 'AGENT_NETWORK_MAX_PAUSES=1\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "network-down", "resume": "network-down"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" paused
  wait_grep "unreachable" "$HOME/notifications.log"
  local pid; pid=$(cat "$STUB_UNITS/agent-$t-r1.service.pid")
  for _ in $(seq 1 100); do   # its runner has ended: it writes the task's fields until then
    kill -0 "$pid" 2> /dev/null || break   # "no such process" is the answer waited for
    sleep 0.1
  done
  sed -i 's/^WAKE_AT=.*/WAKE_AT=1/' "$AGENTS_HOME/tasks/$t/meta.env"   # the timer fires after the pause
  agent _wake "$t"
  wait_status "$t" failed
  wait_grep "unreachable 2 times in a row" "$HOME/notifications.log"   # the reason, as the human gets it
}

@test "a reset or refused connection is a network pause too; an ordinary error is not" {
  source "$REPO_ROOT/bin/agent"
  d=$BATS_TEST_TMPDIR/t
  mkdir -p "$d"
  printf 'Error: connection reset by peer\n' > "$d/err-1.log"
  [ "$(limit_kind "$d" 1 "")" = network ]
  [ "$(quota_reset_seconds "$d" 1 "")" = $(( AGENT_NETWORK_PAUSE_MIN * 60 + 60 )) ]
  printf 'fetch failed: connect ECONNREFUSED 127.0.0.1:443\n' > "$d/err-1.log"
  [ "$(limit_kind "$d" 1 "")" = network ]
  printf 'Error: invalid argument\n' > "$d/err-1.log"
  run limit_kind "$d" 1 ""
  [ "$status" -eq 1 ]
}
