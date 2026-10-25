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

# node_job_setup — NODE_REPOS maps "repo" to the test clone and the clone has two commits. The caller sources
# bin/agent itself: a source inside this function would make agents.conf's declare -A local to it.
node_job_setup() {
  # shellcheck disable=SC2034 # used by prepare_job_task in bin/agent
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  cd "$HOME/repo" || return 1
  FIRST=$(git rev-parse HEAD)
  git -c user.name=t -c user.email=t@t commit -q --allow-empty -m second
  git push -q origin main
}

# run_prepare args... — prepare_job_task in a subshell that closes fd 10: bats itself holds fd 10 as a pipe, so the
# task-ID lock (exec 10>) cannot take it inside a test. Prints NEW_ID on success.
run_prepare() {
  ( exec 10>&-; prepare_job_task "$@" && printf '%s\n' "$NEW_ID" )
}

@test "a job naming a repository outside NODE_REPOS is refused before any task exists" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  run run_prepare job-1 solo other main "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  [ ! -d "$AGENTS_HOME/worktrees/other" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
}

@test "a job whose base ref does not exist fails with a logged reason and starts nothing" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  run run_prepare job-2 solo repo no-such-branch "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
}

@test "a job pinned to a commit starts its worktree at that commit even when the branch moved on" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  id=$(run_prepare job-3 solo repo main "$FIRST" "text")
  [ "$(git -C "$AGENTS_HOME/worktrees/repo/$id" rev-parse HEAD)" = "$FIRST" ]
}

@test "prepare_job_task refuses an unknown agent without creating task or worktree" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  run run_prepare job-unknown ghost repo main "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  [ ! -d "$AGENTS_HOME/worktrees/repo" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
  grep '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl" | grep -q 'unknown agent'
}

@test "prepare_job_task refuses an investigator agent without creating task or worktree" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  run run_prepare job-inv inv repo main "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  [ ! -d "$AGENTS_HOME/worktrees/repo" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
  grep '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl" | grep -q 'investigator'
}

@test "prepare_job_task refuses an agent busy with another task" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  mkdir -p "$TASKS/existing"
  cat > "$TASKS/existing/meta.env" <<EOF
ID=existing
AGENT=solo
STATUS=sleeping
EOF
  run run_prepare job-busy solo repo main "" "text"
  [ "$status" -ne 0 ]
  [ "$(ls -1 "$TASKS" | wc -l)" -eq 1 ]
  [ ! -d "$AGENTS_HOME/worktrees/repo" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
  grep '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl" | grep -q 'busy'
}

@test "prepare_job_task fails cleanly when worktree creation fails" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  touch "$HOME/repo/.git/worktrees"
  run run_prepare job-wt solo repo main "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
  ! grep -q '"event":"task_created"' "$HOME/agents/logs/agent.jsonl"
}

@test "create_task fails and leaves no task when the worktree directory cannot be created" {
  source "$REPO_ROOT/bin/agent"
  cd "$HOME/repo" || return 1
  touch "$AGENTS_HOME/worktrees/repo"   # a file where the worktree directory must be
  mkdir -p "$TASKS/20261025-000000-solo"
  run create_task solo "text" "$HOME/repo" "$HOME/repo" main 20261025-000000 "" HEAD
  [ "$status" -ne 0 ]
  [ ! -e "$TASKS/20261025-000000-solo" ]
  ! grep -q '"event":"task_created"' "$HOME/agents/logs/agent.jsonl"
}

@test "create_task fails and leaves no task when git cannot add the worktree" {
  source "$REPO_ROOT/bin/agent"
  cd "$HOME/repo" || return 1
  mkdir -p "$AGENTS_HOME/worktrees/repo"
  touch "$AGENTS_HOME/worktrees/repo/20261025-000000-solo"   # the worktree path is taken
  mkdir -p "$TASKS/20261025-000000-solo"
  run create_task solo "text" "$HOME/repo" "$HOME/repo" main 20261025-000000 "" HEAD
  [ "$status" -ne 0 ]
  [ ! -e "$TASKS/20261025-000000-solo" ]
  run git -C "$HOME/repo" show-ref --verify --quiet refs/heads/agents/solo/20261025-000000
  [ "$status" -eq 1 ]
  ! grep -q '"event":"task_created"' "$HOME/agents/logs/agent.jsonl"
}

@test "a job whose task worktree cannot be created fails with node_checkout_failed and starts nothing" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  touch "$AGENTS_HOME/worktrees/repo"   # the worktree directory cannot be made
  run run_prepare job-7 solo repo main "" "text"
  [ "$status" -ne 0 ]
  [ -z "$(ls -A "$TASKS")" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
  ! grep -q '"event":"task_created"' "$HOME/agents/logs/agent.jsonl"
}

@test "a job for an unknown agent is refused without an unbound-variable crash" {
  source "$REPO_ROOT/bin/agent"
  node_job_setup
  run run_prepare job-9 ghost repo main "" "text"
  [ "$status" -ne 0 ]
  [[ $output != *unbound* ]]
  [ -z "$(ls -A "$TASKS")" ]
  [ ! -d "$AGENTS_HOME/worktrees/repo" ] || [ -z "$(ls -A "$AGENTS_HOME/worktrees/repo")" ]
  grep -q '"event":"node_checkout_failed"' "$HOME/agents/logs/agent.jsonl"
}
