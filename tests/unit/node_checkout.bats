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
