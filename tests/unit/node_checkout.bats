#!/usr/bin/env bats
# A node job starts its worktree at the commit the checkout resolved, not at the clone's HEAD. create_task is
# the one function both `agent run` and node jobs use; without a start commit it still starts at HEAD.

load helpers

@test "create_task starts the worktree at the given commit, not at HEAD" {
  source "$REPO_ROOT/bin/agent"
  cd "$HOME/repo" || return 1
  old=$(git rev-parse HEAD)
  git -c user.name=t -c user.email=t@t commit -q --allow-empty -m second
  mkdir -p "$TASKS/20261025-000000-solo"
  create_task solo "text" "$HOME/repo" "$HOME/repo" main 20261025-000000 "" "$old"
  [ "$(git -C "$AGENTS_HOME/worktrees/repo/20261025-000000-solo" rev-parse HEAD)" = "$old" ]
}

# node_job_setup — sources the script with NODE_REPOS mapping "repo" to the test clone.
node_job_setup() {
  source "$REPO_ROOT/bin/agent"
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  cd "$HOME/repo" || return 1
  FIRST=$(git rev-parse HEAD)
  git -c user.name=t -c user.email=t@t commit -q --allow-empty -m second
  git push -q origin main
}

@test "a job naming a repository outside NODE_REPOS is refused before any task exists" {
  node_job_setup
  run prepare_job_task job-1 solo other main "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  [ ! -d "$AGENTS_HOME/worktrees/other" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
}

@test "a job whose base ref does not exist fails with a logged reason and starts nothing" {
  node_job_setup
  run prepare_job_task job-2 solo repo no-such-branch "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
}

@test "a job pinned to a commit starts its worktree at that commit even when the branch moved on" {
  node_job_setup
  prepare_job_task job-3 solo repo main "$FIRST" "text"
  [ "$(git -C "$AGENTS_HOME/worktrees/repo/$NEW_ID" rev-parse HEAD)" = "$FIRST" ]
}
