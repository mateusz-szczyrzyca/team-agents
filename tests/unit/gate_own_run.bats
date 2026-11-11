#!/usr/bin/env bats
# Two ways the harness stood in the way of a coordinator's own gate. The runner ended a coordinator's run for working
# alone while its gate ran (agent gate --wait): the gate was killed with the run (exit 143), and the next run began by
# running it again. And an agent's gate of its own uncommitted tree is refused, but a coordinator checks the tree it
# merged from its members before it commits the merge: refused, it ran the checks by hand and wrote the workaround into
# its PR. Now neither cut ends a run while its task's gate holds the gate lock or waits in line for it, and a merge in
# progress may be gated.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "gate_busy_for: the task's gate holds the gate lock or waits in line for it; another task's or an ended one does not" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  sleep 300 & holder=$!
  printf 'label=gate\ntask=T\ndir=/d\npid=%s\npgid=\nstart=%s\n' "$holder" "$(date +%s)" > "$AGENTS_HOME/gate.lock"
  gate_busy_for T
  run gate_busy_for U
  [ "$status" -ne 0 ]
  mkdir -p "$AGENTS_HOME/gate.queue"
  sleep 300 & waiter=$!
  printf 'U\n' > "$AGENTS_HOME/gate.queue/$(date +%s%N)-$waiter"
  gate_busy_for U
  kill "$holder" "$waiter"
  wait "$holder" "$waiter" 2> /dev/null || true
  run gate_busy_for T
  [ "$status" -ne 0 ]   # the holder's line stays in gate.lock after its run: a dead pid holds nothing
  run gate_busy_for U
  [ "$status" -ne 0 ]
}

@test "a coordinator working alone is not ended while its own gate runs; once the gate is over, it is" {
  export AGENT_SOLO_COORD_STEPS=3 AGENT_HANG_POLL=1
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "done"}}'
  agent run --team lead,member "team task" > /dev/null
  t=$(task_of lead); m=$(task_of member)
  wait_status "$m" 'done'
  wait_status "$t" running
  team=$(meta "$t" TEAM)
  flock "$AGENTS_HOME/bus/bus.lock" bash -c 'tmp=$(mktemp "$1.XXXXXX") && jq --arg t "$2" ".teams[\$t].lead = \"lead\"" "$1" > "$tmp" && mv "$tmp" "$1"' \
    _ "$AGENTS_HOME/bus/state.json" "$team"
  sleep 300 & gate=$!
  printf 'label=gate\ntask=%s\ndir=/d\npid=%s\npgid=\nstart=%s\n' "$t" "$gate" "$(date +%s)" > "$AGENTS_HOME/gate.lock"
  for i in 1 2 3 4; do jq -nc --arg i "$i" '{ts: "2026-11-11 10:00:00", k: "cmd", t: ("ls " + $i)}' >> "$AGENTS_HOME/tasks/$t/activity.jsonl"; done
  sleep 4   # some polls of the watch, with the gate on
  run grep -q "worked alone" "$AGENTS_HOME/tasks/$t/activity.jsonl"
  [ "$status" -ne 0 ]
  kill "$gate"
  wait "$gate" 2> /dev/null || true
  wait_grep "worked alone" "$AGENTS_HOME/tasks/$t/activity.jsonl"
  agent stop --running > /dev/null
}

@test "an agent's gate of its own task during a merge in progress runs: the merged tree is checked before its commit" {
  printf "AGENT_REVIEW_GATE='%s'\n" "touch $BATS_TEST_TMPDIR/ran" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
  wt=$(meta "$t" WORKTREE)
  g() { git -C "$wt" -c user.name=t -c user.email=t@t "$@"; }
  g checkout -q -b member-work
  echo m > "$wt/member.txt"
  g add member.txt
  g commit -q -m member
  g checkout -q -
  g merge -q --no-ff --no-commit member-work
  AGENT_RUN_TASK=$t agent gate "$t" > /dev/null
  [ -e "$BATS_TEST_TMPDIR/ran" ]
  [ "$(jq -r '.clean' "$AGENTS_HOME/gates/$t.json")" = false ]   # recorded as what it is: not a pass of HEAD
  agent stop --running > /dev/null
}

@test "the agents' prompt: a merge in progress may be gated before its commit" {
  grep -q "merge in progress" "$REPO_ROOT/prompts/tdd-rounds.md"
}
