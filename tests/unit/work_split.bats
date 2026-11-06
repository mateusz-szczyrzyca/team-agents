#!/usr/bin/env bats
# A fix pass's findings reached every member: each fixed them all, and the coordinator a third time — two members'
# work (25 commits) was thrown away. What the checks and reviews send back now reaches the coordinator alone; a
# member is told to take only the board items assigned to it. The human's addition still reaches everyone, with the
# same line for a member. And the prompts make the board the round's plan: one owner an item — a worker for a small
# one —, its area, its size, when it is done; the coordinator integrates each item as it is done and redoes none.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# ended_round — a team whose coordinator ended its run (the team closed); its member done too
ended_round() {
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$t" 'done'
  wait_status "$m" 'done'
  team=$(meta "$t" TEAM)
}

@test "what the checks and reviews send back reaches the coordinator alone; a member takes only its board items" {
  ended_round
  (source "$REPO_ROOT/bin/agent"; handback "$t" "the fix pass" "AGENT_FIX_PASS=1" run --resume "$team" --same-round "Fix finding M1 in bin/agent.")
  t2=$(task_of lead); m2=$(task_of member)
  grep -q "Fix finding M1 in bin/agent." "$AGENTS_HOME/tasks/$t2/task.md"
  ! grep -q "Fix finding M1 in bin/agent." "$AGENTS_HOME/tasks/$m2/task.md" || false
  grep -q "take only the board items assigned to you" "$AGENTS_HOME/tasks/$m2/task.md"
  agent stop --running > /dev/null
}

@test "the human's addition reaches every member, a member with the same line" {
  ended_round
  agent run --resume "$team" --same-round "The human cannot answer until tomorrow." > /dev/null
  t2=$(task_of lead); m2=$(task_of member)
  grep -q "The human cannot answer until tomorrow." "$AGENTS_HOME/tasks/$t2/task.md"
  grep -q "The human cannot answer until tomorrow." "$AGENTS_HOME/tasks/$m2/task.md"
  grep -q "take only the board items assigned to you" "$AGENTS_HOME/tasks/$m2/task.md"
  agent stop --running > /dev/null
}

@test "the team prompt makes the board the round's plan, and a member take only its items" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  p=$AGENTS_HOME/tasks/$(task_of lead)/prompt-1.md
  for _ in $(seq 1 300); do grep -q "The coordinator" "$p" 2> /dev/null && break; sleep 0.2; done
  grep -q "The board is the round's plan" "$p"
  grep -q "its area" "$p"
  grep -q "You integrate; you redo nothing" "$p"
  grep -q "Only your board items" "$p"
  agent stop --running > /dev/null
}
