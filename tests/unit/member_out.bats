#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# A member out of the team's work — a failed run, a long pause, a dead runner — gives its board items back, with
# its progress notes, and the team is told who, why, which items and where its work is. An unknown STATUS first
# goes back to the agent once.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# item team field — a field of board item B1
item() { agentbus team-get --dir "$AGENTS_HOME/bus" --team "$1" | jq -r ".board.B1.$2"; }

@test "an unknown STATUS goes back to the agent once, with the statuses it may use" {
  scenario '{"solo": {"new": "blocked", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  [ "$(meta "$t" RUN)" = 2 ]
  grep -q 'STATUS: BLOCKED' "$HOME/prompt-solo-resume.txt"
  grep -q 'SLEEPING' "$HOME/prompt-solo-resume.txt"
}

@test "a second unknown STATUS fails the run" {
  scenario '{"solo": {"new": "blocked", "resume": "blocked"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" failed
  [ "$(meta "$t" RUN)" = 2 ]
}

@test "a failed member's items go back with its progress notes, and the team is told where its work is" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "claim-fail"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" failed
  team=$(meta "$m" TEAM)
  wait_grep 'MEMBER member IS OUT' "$AGENTS_HOME/bus/messages.jsonl"
  grep 'MEMBER member IS OUT' "$AGENTS_HOME/bus/messages.jsonl" | grep -q 'B1'
  [ "$(item "$team" status)" = todo ]
  [ "$(item "$team" previous)" = member ]
  [[ $(item "$team" progress) == *"lexer done at abc123"* ]]
}

@test "a long pause gives the member's items back; a short one keeps them" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "claim-quota"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" paused
  team=$(meta "$m" TEAM)
  wait_grep 'back on the board' "$AGENTS_HOME/bus/messages.jsonl"   # the items go back right after the pause
  [ "$(item "$team" status)" = todo ]
}

@test "a short pause (a rate limit) keeps the member's items" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "claim-rate-limit"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" paused
  team=$(meta "$m" TEAM)
  [ "$(item "$team" status)" = claimed ]
}

@test "agent _supervise finds a task whose runner died and ends it" {
  scenario '{"solo": {"new": "sleep:60"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" running
  # the runner dies; as systemd would, the stub then sees no active unit (a killed process can linger as a zombie in
  # a container, which kill -0 still finds — so the recorded PID is replaced by one that does not exist)
  pidfile=$(ls "$STUB_UNITS"/*"$t"*.pid)
  kill -9 -- "-$(cat "$pidfile")" 2> /dev/null || kill -9 "$(cat "$pidfile")"
  echo 999999 > "$pidfile"
  AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  [ "$(meta "$t" STATUS)" = failed ]
  wait_grep 'runner died' "$HOME/notifications.log"
}
