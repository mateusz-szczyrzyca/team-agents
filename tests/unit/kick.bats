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

@test "kicking a team member ends its run without leaving the team: it sleeps or is woken again, the coordinator is told" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "sleep:60", "resume": "done"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" running
  run agent kick "$m"
  [ "$status" -eq 0 ]
  # it went to sleep; with messages waiting for it, the bus wakes it at once for a fresh run — it is not out
  [[ $(meta "$m" STATUS) =~ ^(sleeping|starting|running)$ ]]
  team=$(meta "$m" TEAM)
  [[ $(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.members.member.state') =~ ^(sleeping|waking|active)$ ]]
  [ -n "$(meta "$m" KICKED)" ]
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

@test "kicking a team member after its team is done marks it stopped, not sleeping" {
  scenario '{"lead": {"new": "sleep:60"}, "member": {"new": "sleep:60"}}'
  agent run --team lead,member "team task"
  l=$(task_of lead)
  m=$(task_of member)
  wait_status "$l" running
  wait_status "$m" running
  team=$(meta "$m" TEAM)
  agentbus team-done --dir "$AGENTS_HOME/bus" --team "$team"
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.done')" = true ]

  run agent kick "$m" web
  [ "$status" -eq 0 ]
  [ "$(meta "$m" STATUS)" = stopped ]
  [ -n "$(meta "$m" FINISHED)" ]
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.members.member.state')" = stopped ]
}

@test "stopping a team member on demand marks its bus member state as stopped, not sleeping" {
  scenario '{"lead": {"new": "sleep:60"}, "member": {"new": "sleep:60"}}'
  agent run --team lead,member "team task"
  l=$(task_of lead)
  m=$(task_of member)
  wait_status "$l" running
  wait_status "$m" running
  team=$(meta "$m" TEAM)

  run agent stop "$m"
  [ "$status" -eq 0 ]
  [ "$(meta "$m" STATUS)" = stopped ]
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.members.member.state')" = stopped ]

  run agent stop --running
  [ "$status" -eq 0 ]
  [ "$(meta "$l" STATUS)" = stopped ]
  [ "$(agentbus team-get --dir "$AGENTS_HOME/bus" --team "$team" | jq -r '.members.lead.state')" = stopped ]
}

