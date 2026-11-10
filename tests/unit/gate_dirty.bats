#!/usr/bin/env bats
# A member ran the full gate (some 4.5 minutes) after every small change, about fifteen times in eighty minutes, most
# on uncommitted work — runs that count for nothing: board_done needs a pass of a committed HEAD. The team waited on it.
# An agent's gate of its own task (AGENT_RUN_TASK, which the runner now gives the agent's commands) on an uncommitted
# tree is refused, with what to do instead — single tests and the linter while working, one gate on the commit — and
# the refusal is kept in the gates' history. A human's gate, or one of another task, runs as before. agent debug
# counts a task's gate runs, those on an uncommitted tree and the refused ones.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

# with_task gate — a solo task waiting (on its questions) in its worktree, the gate's command set; sets t and wt
with_task() {
  printf "AGENT_REVIEW_GATE='%s'\n" "$1" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" waiting
  wt=$(meta "$t" WORKTREE)
}

@test "an agent's gate of its own task on an uncommitted tree is refused, with what to do; nothing runs" {
  with_task "touch $BATS_TEST_TMPDIR/ran"
  echo x > "$wt/new.txt"
  AGENT_RUN_TASK=$t run agent gate "$t"
  [ "$status" -eq 2 ]
  [[ $output == *"not run — the tree has uncommitted changes"* ]]
  [[ $output == *"single tests"* ]]
  [[ $output == *"new.txt"* ]]   # what makes it unclean
  [ ! -e "$BATS_TEST_TMPDIR/ran" ]
  [ "$(jq -r --arg t "$t" 'select(.task == $t) | .status' "$AGENTS_HOME/gates/history.jsonl")" = refused ]
  [ ! -e "$AGENTS_HOME/gates/$t.json" ]   # the last run — what gate-status and board_done read — untouched
  agent stop --running > /dev/null
}

@test "a human's gate, or one of another task, on an uncommitted tree runs as before; an agent's on a commit runs" {
  with_task "touch $BATS_TEST_TMPDIR/ran"
  echo x > "$wt/new.txt"
  agent gate "$t" > /dev/null
  [ -e "$BATS_TEST_TMPDIR/ran" ] && rm "$BATS_TEST_TMPDIR/ran"
  AGENT_RUN_TASK=20261110-000001-other agent gate "$t" > /dev/null
  [ -e "$BATS_TEST_TMPDIR/ran" ] && rm "$BATS_TEST_TMPDIR/ran"
  git -C "$wt" add new.txt
  git -C "$wt" -c user.name=t -c user.email=t@t commit -q -m c
  AGENT_RUN_TASK=$t agent gate "$t" > /dev/null
  [ -e "$BATS_TEST_TMPDIR/ran" ]
  agent stop --running > /dev/null
}

@test "the runner gives the agent's commands AGENT_RUN_TASK, its task" {
  scenario '{"solo": {"new": "say-run-task"}}'
  agent run solo "a task" > /dev/null
  t=$(task_of solo)
  wait_status "$t" 'done'
  grep -q "RUN_TASK=$t" "$AGENTS_HOME/tasks/$t/final.md"
}

@test "agent debug counts a task's gate runs, those on an uncommitted tree, and the refused ones" {
  with_task 'true'
  echo x > "$wt/new.txt"
  agent gate "$t" > /dev/null
  AGENT_RUN_TASK=$t agent gate "$t" > /dev/null || true
  git -C "$wt" add new.txt
  git -C "$wt" -c user.name=t -c user.email=t@t commit -q -m c
  agent gate "$t" > /dev/null
  agent debug "$t" --out "$BATS_TEST_TMPDIR/d.txt" > /dev/null
  grep -q "gate runs: 2 (on an uncommitted tree: 1); refused on an uncommitted tree: 1" "$BATS_TEST_TMPDIR/d.txt"
  agent stop --running > /dev/null
}

@test "the agents' prompt: agent gate refuses an uncommitted tree; single tests and the linter while working" {
  grep -q "refuses an uncommitted tree" "$REPO_ROOT/prompts/tdd-rounds.md"
}
