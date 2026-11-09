#!/usr/bin/env bats
# The harness's side of the gate at board_done: agent _board_done_check <task> answers the bus — exit 0, the task may
# submit an item's result; exit 3, its work changed code (since its round's base) and no gate pass of its HEAD on a
# clean tree is recorded (agent gate). Any other failure is no refusal: the bus lets the result through, logged.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# with_task gate — a solo task waiting in its worktree, the gate's command set; sets t and wt
with_task() {
  printf "AGENT_REVIEW_GATE='%s'\n" "$1" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
  wt=$(meta "$t" WORKTREE)
}

commit_code() {
  printf 'package main\n\nfunc g() int { return 2 }\n' > "$wt/g.go"
  git -C "$wt" add g.go
  git -C "$wt" -c user.name=t -c user.email=t@t commit -qm "code"
}

@test "work without a change of code needs no gate: exit 0" {
  with_task 'true'
  run agent _board_done_check "$t"
  [ "$status" -eq 0 ]
  agent stop --running > /dev/null
}

@test "a change of code without a gate pass of HEAD is refused (exit 3) and says what to do; after agent gate, passes" {
  with_task 'true'
  commit_code
  run agent _board_done_check "$t"
  [ "$status" -eq 3 ]
  [[ $output == *"agent gate"* && $output == *"gate_exception"* ]]
  agent gate "$t" > /dev/null
  run agent _board_done_check "$t"
  [ "$status" -eq 0 ]
  [[ $output == *PASS* ]]
  agent stop --running > /dev/null
}

@test "a repository without a gate is not held: exit 0" {
  with_task 'off'
  sed -i "s/^AGENT_REVIEW_GATE=.*/AGENT_REVIEW_GATE=off/" "$AGENTS_HOME/agents.conf"
  commit_code
  run agent _board_done_check "$t"
  [ "$status" -eq 0 ]
  agent stop --running > /dev/null
}

@test "the protocol tells agents that a code item's result needs its gate, and when gate_exception is right" {
  grep -q "board_done" "$REPO_ROOT/prompts/protocol.md"
  grep -q "gate_exception" "$REPO_ROOT/prompts/protocol.md"
}
