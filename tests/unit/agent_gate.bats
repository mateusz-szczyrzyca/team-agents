#!/usr/bin/env bats
# A "gate PASS" in an agent's result could not be checked, and the test lock lived in one repository's Makefile.
# agent gate runs the repository's gate (as the precheck finds it: AGENT_REVIEW_GATE, by default make gate where the
# repository has one) in a task's worktree, one run at a time on the machine, and records what happened in
# AGENTS_HOME: the commit, whether the tree was clean, the result. agent gate-status says whether the last record
# is a pass of the task's current HEAD on a clean tree — the fact an agent's claim can be checked against.

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

commit_in() { git -C "$wt" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "$1"; }

@test "a passing gate is recorded for the task's HEAD on a clean tree; gate-status says so and exits 0" {
  with_task 'echo all good'
  run agent gate "$t"
  [ "$status" -eq 0 ]
  [[ $output == *"all good"* && $output == *PASS* ]]
  run agent gate-status "$t"
  [ "$status" -eq 0 ]
  [[ $output == *PASS* && $output == *"$(git -C "$wt" rev-parse --short HEAD)"* ]]
  agent stop --running > /dev/null
}

@test "a failing gate is recorded as FAIL with its exit status; gate-status exits non-zero" {
  with_task 'echo "3 tests failed"; exit 2'
  run agent gate "$t"
  [ "$status" -eq 2 ]
  [[ $output == *"3 tests failed"* && $output == *FAIL* ]]
  run agent gate-status "$t"
  [ "$status" -ne 0 ]
  [[ $output == *FAIL* ]]
  agent stop --running > /dev/null
}

@test "a pass on a dirty tree, or of an older commit, is not a pass of the task's HEAD" {
  with_task 'true'
  echo change > "$wt/uncommitted.txt"
  agent gate "$t" > /dev/null
  run agent gate-status "$t"
  [ "$status" -ne 0 ]
  [[ $output == *"not clean"* ]]
  rm "$wt/uncommitted.txt"
  agent gate "$t" > /dev/null
  agent gate-status "$t" > /dev/null
  commit_in "after the gate"
  run agent gate-status "$t"
  [ "$status" -ne 0 ]
  [[ $output == *"no gate run for HEAD"* ]]
  agent stop --running > /dev/null
}

@test "without a task, agent gate and gate-status take the task whose worktree they run in" {
  with_task 'true'
  cd "$wt"
  run agent gate
  [ "$status" -eq 0 ]
  run agent gate-status
  [ "$status" -eq 0 ]
  [[ $output == *PASS* ]]
  cd /
  run agent gate
  [ "$status" -ne 0 ]
  agent stop --running > /dev/null
}

@test "one gate run at a time on the machine: a second refuses with the message every agent knows, exit 75" {
  with_task 'sleep 5'
  agent gate "$t" > /dev/null &
  first=$!
  for _ in $(seq 1 50); do grep -qs '^pid=' "$AGENTS_HOME/gate.lock" && break; sleep 0.1; done
  run agent gate "$t"
  [ "$status" -eq 75 ]
  [[ $output == *"another tests are running now - please wait before you try again"* ]]
  wait "$first"
  run agent gate "$t" --wait   # --wait: in line instead of refusing
  [ "$status" -eq 0 ]
  agent stop --running > /dev/null
}

@test "a gate run alive past AGENT_GATE_STALE_MIN is taken for hung: stopped, and the lock taken over" {
  with_task 'sleep 3017'   # a duration no other test uses: the files run in parallel
  agent gate "$t" > /dev/null 2>&1 &
  for _ in $(seq 1 50); do grep -qs '^pid=' "$AGENTS_HOME/gate.lock" && break; sleep 0.1; done
  sed -i "s/^AGENT_REVIEW_GATE=.*/AGENT_REVIEW_GATE='true'/" "$AGENTS_HOME/agents.conf"
  AGENT_GATE_STALE_MIN=0 run agent gate "$t"
  [ "$status" -eq 0 ]
  [[ $output == *hung* ]]
  ! pgrep -fx "sleep 3017" > /dev/null || false
  agent stop --running > /dev/null
}

@test "the prompts send agents to agent gate, not to one repository's make gate" {
  grep -q "agent gate" "$REPO_ROOT/prompts/protocol.md"
  ! grep -q "GATE_WAIT=1 make gate" "$REPO_ROOT/prompts/protocol.md" || false
  ! grep -q "in team-agents" "$REPO_ROOT/prompts/tdd-rounds.md" || false
}
