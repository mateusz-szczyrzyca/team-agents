#!/usr/bin/env bats
# Every message to a sleeping member starts a whole new run of it. The team prompt says so with its consequences: the
# coordinator gives work through the board in batches, writes once with all a member needs, and sends notes quietly;
# a member does not answer with acknowledgements.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "the team prompt says what a message to a sleeping member costs, and how to spare it" {
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  p=$AGENTS_HOME/tasks/$m/prompt-1.md
  for _ in $(seq 1 300); do grep -q "The member's role" "$p" 2> /dev/null && break; sleep 0.2; done
  grep -q "starts a whole new run" "$p"
  grep -q "in batches" "$p"
  grep -q "acknowledgements" "$p"
  wait_status "$m" sleeping 60
  agent team "$(meta "$m" TEAM)" --close > /dev/null
}
