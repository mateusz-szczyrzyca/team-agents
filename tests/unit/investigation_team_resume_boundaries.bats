#!/usr/bin/env bats
# Tests for advisory resume boundaries: partial verdict failures, missing PR refusal,
# and stop cleanup of untagged manifest children.

load helpers

# field task_id key reads metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# meta_put task key value updates or appends key=value in a task's meta.env.
meta_put() {
  local file="$AGENTS_HOME/tasks/$1/meta.env"
  if grep -q "^$2=" "$file"; then sed -i "s|^$2=.*|$2=$3|" "$file"; else printf '%s=%s\n' "$2" "$3" >> "$file"; fi
}

# configure_investigators sets up inv and inv2 in agents.conf.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' \
    "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
}

# setup_parent_resume id team pr_num branch sets up an investigation team parent in phase 5.
setup_parent_resume() {
  local id=$1 team=$2 pr_num=$3 branch=$4
  local dir="$AGENTS_HOME/tasks/$id"
  mkdir -p "$dir"
  cat > "$dir/meta.env" << EOF
ID=$id
AGENT=investigation-team
KIND=investigation_team
STATUS=new
TYPE=internal
MODEL=
EXTRA_ARGS=
TEAM=
TIER=normal
RUN=0
ROUND=0
CREATED=$(date -Is)
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BASE=main
INVESTIGATION_PIPELINE=1
INVESTIGATION_TEAM=$team
INVESTIGATION_TOPIC_SLUG=topic
INVESTIGATION_PHASE=5
INVESTIGATION_BASE_SHA=abc1234
BRANCH=$branch
PR_URL=https://github.com/example/repo/pull/$pr_num
PR_NUMBER=$pr_num
AUTO_STAGE=resuming
EOF
  printf '%s\n' 'Investigate caching' > "$dir/task.md"
}

# recorded_member_json parent agent key returns a field from parent's resume.json for agent.
recorded_member_json() {
  local parent=$1 agent=$2 key=$3
  local file="$AGENTS_HOME/tasks/$parent/resume.json"
  [[ -f $file ]] || return 1
  jq -r --arg a "$agent" --arg k "$key" '.members[] | select(.agent == $a) | .[$k] // ""' "$file"
}

@test "mixed done verdict and failed advisory reviewer formats stage with failure count and no replacement" {
  configure_investigators
  local parent="20261102-000030-investigation-team"
  setup_parent_resume "$parent" "inv,inv2" "42" "agents/inv/integration"
  meta_put "$parent" STATUS running
  meta_put "$parent" INVESTIGATION_LEASE 1

  local child_inv="20261102-000031-inv"
  local child_inv2="20261102-000032-inv2"
  meta_put "$parent" INVESTIGATION_RESUME_TASKS "$child_inv,$child_inv2"

  cat > "$AGENTS_HOME/tasks/$parent/resume.json" << EOF
{
  "version": 1,
  "members": [
    {"agent": "inv", "task": "$child_inv", "status": "running"},
    {"agent": "inv2", "task": "$child_inv2", "status": "running"}
  ]
}
EOF

  mkdir -p "$AGENTS_HOME/tasks/$child_inv"
  cat > "$AGENTS_HOME/tasks/$child_inv/meta.env" << EOF
ID=$child_inv
AGENT=inv
KIND=investigation
INVESTIGATION_TEAM=$parent
INVESTIGATION_PHASE=5
REVIEW=1
STATUS=done
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF
  printf '%s\n' 'VERDICT: ready-to-merge' > "$AGENTS_HOME/tasks/$child_inv/final.md"

  mkdir -p "$AGENTS_HOME/tasks/$child_inv2"
  cat > "$AGENTS_HOME/tasks/$child_inv2/meta.env" << EOF
ID=$child_inv2
AGENT=inv2
KIND=investigation
INVESTIGATION_TEAM=$parent
INVESTIGATION_PHASE=5
REVIEW=1
STATUS=failed
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF

  run agent dispatch
  [ "$status" -eq 0 ]

  # Parent transitions to failed without spawning replacements; lease released
  [ "$(field "$parent" STATUS)" = "failed" ]
  [ "$(field "$parent" INVESTIGATION_LEASE)" != "1" ]

  # Stage includes exact failure suffix
  [[ "$(field "$parent" AUTO_STAGE)" == *"failed: advisory reviews ready (#42, 1 failed)"* ]]

  # Manifest records individual member status and verdict
  [ "$(recorded_member_json "$parent" inv status)" = "done" ]
  [ "$(recorded_member_json "$parent" inv verdict)" = "ready-to-merge" ]
  [ "$(recorded_member_json "$parent" inv2 status)" = "failed" ]
  [ "$(find "$AGENTS_HOME/tasks" -name meta.env | wc -l)" -eq 3 ]
}

@test "resuming an investigation team without a pull request is refused fail-fast" {
  configure_investigators
  local parent="20261102-000040-investigation-team"
  setup_parent_resume "$parent" "inv,inv2" "" "agents/inv/integration"
  meta_put "$parent" STATUS waiting
  meta_put "$parent" INVESTIGATION_PHASE 4
  meta_put "$parent" PR_URL ""
  meta_put "$parent" PR_NUMBER ""

  run agent run --resume "$parent"
  [ "$status" -ne 0 ]
  [[ $output == *"no pull request"* || $output == *"new topic"* ]]

  # Refusal must not create new child tasks
  [ "$(find "$AGENTS_HOME/tasks" -name meta.env | wc -l)" -eq 1 ]
}

@test "agent stop on phase 5 parent stops untagged child referenced by resume manifest before releasing lease" {
  configure_investigators
  local parent="20261102-000050-investigation-team"
  setup_parent_resume "$parent" "inv" "42" "agents/inv/integration"
  meta_put "$parent" STATUS running
  meta_put "$parent" INVESTIGATION_LEASE 1

  local child="20261102-000051-inv"
  meta_put "$parent" INVESTIGATION_RESUME_TASKS "$child"

  cat > "$AGENTS_HOME/tasks/$parent/resume.json" << EOF
{
  "version": 1,
  "members": [
    {"agent": "inv", "task": "$child", "status": "running"}
  ]
}
EOF

  # Child directory created but not yet tagged with INVESTIGATION_TEAM
  mkdir -p "$AGENTS_HOME/tasks/$child"
  cat > "$AGENTS_HOME/tasks/$child/meta.env" << EOF
ID=$child
AGENT=inv
KIND=worker
STATUS=running
RUN=1
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF

  run agent stop "$parent"
  [ "$status" -eq 0 ]

  # Parent status is stopped and lease is released
  [ "$(field "$parent" STATUS)" = "stopped" ]
  [ -z "$(field "$parent" INVESTIGATION_LEASE)" ]

  # Child referenced by resume manifest must be stopped even without INVESTIGATION_TEAM tag
  [ "$(field "$child" STATUS)" = "stopped" ]
}
