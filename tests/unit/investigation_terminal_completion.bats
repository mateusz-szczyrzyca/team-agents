#!/usr/bin/env bats
# Terminal handoff recovery preserves the result without requiring a human decision.

load helpers

# terminal_parent result creates an interrupted handoff using the pre-change metadata format.
terminal_parent() {
  local result=$1
  TERMINAL_PARENT=20261102-000001-investigation-team
  mkdir -p "$AGENTS_HOME/tasks/$TERMINAL_PARENT"
  cat > "$AGENTS_HOME/tasks/$TERMINAL_PARENT/meta.env" <<META
ID=$TERMINAL_PARENT
AGENT=investigation-team
KIND=investigation_team
TYPE=internal
STATUS=waiting
TEAM=
RUN=0
ROUND=0
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
INVESTIGATION_PIPELINE=1
INVESTIGATION_PHASE=4
INVESTIGATION_LEASE=
INVESTIGATION_HANDOFF_PENDING=1
INVESTIGATION_HANDOFF_STAGE=waits\\ for\\ you:\\ review\\ ready\\ \\(#73\\)
INVESTIGATION_HANDOFF_RESULT=$result
PR_URL=https://github.com/example/repo/pull/73
PR_NUMBER=73
META
}

@test "successful legacy handoff finishes done and notifies completion exactly once" {
  terminal_parent 'done'
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$TERMINAL_PARENT")" = 'done' ]
  grep -q 'Status: ✅ done' "$HOME/notifications.log"
  run grep -Ei 'waits for you|ready for your decision|assess this investigation' "$HOME/notifications.log" "$HOME/gh.log"
  [ "$status" -eq 1 ]
  agent dispatch > "$HOME/terminal-dispatch.log" 2>&1
  [ "$(grep -c '^===' "$HOME/notifications.log")" -eq 1 ]
  [ "$(grep -c 'gh pr comment' "$HOME/gh.log")" -eq 1 ]
}

@test "failed legacy handoff finishes failed and preserves its failure notification" {
  terminal_parent failed
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$TERMINAL_PARENT")" = failed ]
  grep -q 'Status: ❌ failed' "$HOME/notifications.log"
  run grep -Ei 'waits for you|ready for your decision|assess this investigation' "$HOME/notifications.log" "$HOME/gh.log"
  [ "$status" -eq 1 ]
}
