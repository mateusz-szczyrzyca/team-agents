#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# agent kick (the web interface's stop button): cut an agent's current run short. A team member goes to sleep —
# its team can wake it again — and the coordinator is told; a solo task is stopped. Both are recorded.

load helpers

# meta id key — a value from a task's meta.env
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "kicking a solo task stops its run and records it" {
  scenario '{"solo": {"new": "sleep:60"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" running
  run agent kick "$t" web
  [ "$status" -eq 0 ]
  [ "$(meta "$t" STATUS)" = stopped ]
  [ -n "$(meta "$t" KICKED)" ]
  grep -q 'stopped by the human' "$AGENTS_HOME/tasks/$t/activity.jsonl"
}

@test "kicking a team member puts it to sleep, so the team can wake it, and tells the coordinator" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "sleep:60", "resume": "done"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" running
  run agent kick "$m"
  [ "$status" -eq 0 ]
  [ "$(meta "$m" STATUS)" = sleeping ]
  team=$(meta "$m" TEAM)
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.members.member.state')" = sleeping ]
  wait_grep 'was stopped by the human' "$AGENTS_HOME/bus/messages.jsonl"
}

@test "kicking a task that does not run is refused" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  run agent kick "$t"
  [ "$status" -ne 0 ]
  [[ $output == *"not running"* ]]
}
