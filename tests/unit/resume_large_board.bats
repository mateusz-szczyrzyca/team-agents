#!/usr/bin/env bats
# A round's precheck failed and the work could not go back to the team: "jq: Argument list too long". Its team's
# board (69 items, long progress notes) was passed to jq as one argument, past the kernel's limit for one, and the
# resumed team was never created. The old team now reaches jq on its standard input, whatever the board's size.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a team whose board is larger than one argument can hold is resumed, its board carried" {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); team=$(meta "$t" TEAM)
  wait_status "$t" sleeping
  wait_status "$(task_of member)" sleeping
  long=$(head -c 60000 /dev/zero | tr '\0' 'x')
  for i in 1 2 3; do
    "$BUILD_DIR/agentbus" board-add --dir "$AGENTS_HOME/bus" --team "$team" --as lead --title "item $i" --details "$long" --level normal > /dev/null
  done
  agent stop --running > /dev/null
  run agent run --resume "$team" --same-round "go on"
  [ "$status" -eq 0 ]
  [[ $output != *"Argument list too long"* ]]
  new=$(task_of lead)
  [ "$new" != "$t" ]
  newteam=$(meta "$new" TEAM)
  [ "$("$BUILD_DIR/agentbus" board-dump --dir "$AGENTS_HOME/bus" --team "$newteam" | jq '.board | length')" -eq 3 ]
  agent stop --running > /dev/null
}
