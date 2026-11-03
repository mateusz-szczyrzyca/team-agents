#!/usr/bin/env bats
# A round's work stays in its round. A member that ran out of the round's items started the next round's on its own
# branch: in no PR, checked and reviewed with nothing, and left behind when the next round starts on fresh branches.
# The prompts say to end the round instead, and a member to sleep.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "the prompts say never to start the next round's work; a member out of items sleeps instead" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "a task"
  t=$(task_of solo)
  wait_status "$t" 'done'
  grep -qi "never start the next round" "$AGENTS_HOME/tasks/$t/prompt-1.md"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  p=$AGENTS_HOME/tasks/$m/prompt-1.md
  for _ in $(seq 1 300); do grep -q "The member's role" "$p" 2> /dev/null && break; sleep 0.2; done
  grep -q "not the next round's items" "$p"
  wait_status "$m" sleeping 60
  agent team "$(meta "$m" TEAM)" --close > /dev/null
}
