#!/usr/bin/env bats
# Regression for review finding R1:
# An inactive independent investigation child from an earlier boot with exhausted
# restarts must not crash supervision on unbound FROM_AGENT and must allow cohort dispatch.

load helpers

# field decodes the shell-quoted metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# configure_investigators sets up inv and inv2
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' \
    "$AGENTS_HOME/agents.conf"
}

# independent_setup configures two investigators and a bus-free CLI only in this test's temporary home.
independent_setup() {
  configure_investigators
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  printf '%s\n' 'Independent topic' > "$HOME/topic.md"
}

# recorded_status checks the immutable terminal snapshot of one phase-one member.
recorded_status() {
  jq -r --arg a "$2" '.members[] | select(.agent == $a) | .status' "$AGENTS_HOME/tasks/$1/phase1.json"
}

@test "inactive independent investigation child with exhausted restarts fails without crashing supervision" {
  independent_setup
  export AGENT_BOOT_ID_FILE="$BATS_TEST_TMPDIR/boot_id"
  echo "boot-current" > "$AGENT_BOOT_ID_FILE"

  local base parent child1 child2
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  parent="20261101-000000-investigation-team"
  child1="20261101-000001-inv"
  child2="20261101-000002-inv2"
  mkdir -p "$AGENTS_HOME/tasks/$parent" "$AGENTS_HOME/tasks/$child1" "$AGENTS_HOME/tasks/$child2"

  cat > "$AGENTS_HOME/tasks/$parent/meta.env" << EOF
ID=$parent
AGENT=investigation-team
KIND=investigation_team
STATUS=running
TYPE=internal
INVESTIGATION_PHASE=1
INVESTIGATION_TEAM=inv,inv2
INVESTIGATION_LEASE=1
RUN=0
RUNNER_DONE=1
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BASE=$base
INVESTIGATION_BASE_SHA=$base
EOF

  cat > "$AGENTS_HOME/tasks/$child1/meta.env" << EOF
ID=$child1
AGENT=inv
KIND=investigation
INVESTIGATION_TEAM=$parent
INVESTIGATION_PHASE=1
PARENT_TASK=$parent
TYPE=claude
STATUS=running
RUN=1
RUN_BOOT=boot-previous
RESTARTS=2
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BRANCH=agents/inv/test
BASE=$base
EOF

  cat > "$AGENTS_HOME/tasks/$child2/meta.env" << EOF
ID=$child2
AGENT=inv2
KIND=investigation
INVESTIGATION_TEAM=$parent
INVESTIGATION_PHASE=1
PARENT_TASK=$parent
TYPE=claude
STATUS=failed
RUN=1
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BRANCH=agents/inv2/test
BASE=$base
EOF

  cat > "$AGENTS_HOME/tasks/$parent/phase1.json" << EOF
{"version":1,"members":[{"agent":"inv","task":"$child1","branch":"agents/inv/test","status":"running","commit":""},{"agent":"inv2","task":"$child2","branch":"agents/inv2/test","status":"failed","commit":""}]}
EOF

  run env AGENT_SUPERVISE_GRACE_SEC=0 AGENT_BOOT_ID_FILE="$AGENT_BOOT_ID_FILE" agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$child1" STATUS)" = "failed" ]
  [ "$(field "$child1" RESTARTS)" = "3" ]
  # shellcheck source=/dev/null
  (source "$AGENTS_HOME/tasks/$child1/meta.env" && [[ $NOTE == *"restarted 3 times"* ]])
  [ "$(recorded_status "$parent" inv)" = "failed" ]
  [ "$(recorded_status "$parent" inv2)" = "failed" ]
  [ "$(field "$parent" STATUS)" = "waiting" ]
  [ "$(field "$parent" AUTO_STAGE)" = 'waits for you: all phase 1 runs failed' ]
  [ "$(field "$parent" INVESTIGATION_LEASE)" != 1 ]
}

@test "ordinary investigation with exhausted restarts notifies caller and marks task failed" {
  export AGENT_BOOT_ID_FILE="$BATS_TEST_TMPDIR/boot_id"
  echo "boot-current" > "$AGENT_BOOT_ID_FILE"

  local base task
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  task="20261101-000003-inv"
  mkdir -p "$AGENTS_HOME/tasks/$task"

  cat > "$AGENTS_HOME/tasks/$task/meta.env" << EOF
ID=$task
AGENT=inv
KIND=investigation
TYPE=claude
STATUS=running
RUN=1
RUN_BOOT=boot-previous
RESTARTS=2
INV_ID=42
FROM_AGENT=coordinator
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BRANCH=agents/inv/test
BASE=$base
EOF

  run env AGENT_SUPERVISE_GRACE_SEC=0 AGENT_BOOT_ID_FILE="$AGENT_BOOT_ID_FILE" agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$task" STATUS)" = "failed" ]
  [ -f "$AGENTS_HOME/notifications/$task-r1-summary.md" ]
  grep -q "investigation #42 for coordinator" "$AGENTS_HOME/notifications/$task-r1-summary.md"
}
