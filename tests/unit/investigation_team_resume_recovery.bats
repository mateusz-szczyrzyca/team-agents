#!/usr/bin/env bats
# Tests for investigation team Round 4 advisory resume recovery, capacity queueing,
# and interrupted admission.

load helpers

# field task_id key decodes shell-quoted metadata written by meta_set in this isolated test home.
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

# recorded_resume_status reads the status of one member from parent's resume.json.
recorded_resume_status() {
  local parent=$1 agent=$2
  local file="$AGENTS_HOME/tasks/$parent/resume.json"
  [[ -f $file ]] || return 1
  jq -r --arg a "$agent" '.members[] | select(.agent == $a) | .status' "$file"
}

@test "busy member queues whole resume cohort without partial starts" {
  configure_investigators
  local parent="20261102-000000-investigation-team"
  setup_parent_resume "$parent" "inv,inv2" "42" "agents/inv/integration"

  # Make inv2 busy with another running task
  local busy_task="20261102-000001-inv2"
  mkdir -p "$AGENTS_HOME/tasks/$busy_task"
  cat > "$AGENTS_HOME/tasks/$busy_task/meta.env" << EOF
ID=$busy_task
AGENT=inv2
KIND=investigation
STATUS=running
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF

  run agent dispatch
  [ "$status" -eq 0 ]

  # Entire cohort must be queued: parent remains new or waiting without lease, no partial child for inv
  [ "$(field "$parent" STATUS)" = "new" ] || [ "$(field "$parent" STATUS)" = "waiting" ]
  [ "$(field "$parent" INVESTIGATION_LEASE)" != "1" ]
  [ "$(field "$parent" AUTO_STAGE)" = "waiting: capacity queue" ]
  [ -z "$(task_of inv)" ]
}

@test "quota-paused member queues whole resume cohort without partial starts" {
  configure_investigators
  local parent="20261102-000002-investigation-team"
  setup_parent_resume "$parent" "inv,inv2" "42" "agents/inv/integration"

  # Make inv2 paused in provider quota
  local quota_task="20261102-000003-inv2"
  mkdir -p "$AGENTS_HOME/tasks/$quota_task"
  cat > "$AGENTS_HOME/tasks/$quota_task/meta.env" << EOF
ID=$quota_task
AGENT=inv2
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) + 3600 ))
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF

  run agent dispatch
  [ "$status" -eq 0 ]

  # Entire cohort must be queued due to quota pause without partial launch of inv
  [ "$(field "$parent" STATUS)" = "new" ] || [ "$(field "$parent" STATUS)" = "waiting" ]
  [ "$(field "$parent" INVESTIGATION_LEASE)" != "1" ]
  [ "$(field "$parent" AUTO_STAGE)" = "waiting: capacity queue" ]
  [ -z "$(task_of inv)" ]
}

@test "recovered phase 5 leased parent fails uncreated child slots but retains lease while live reviewer runs" {
  configure_investigators
  local parent="20261102-000010-investigation-team"
  setup_parent_resume "$parent" "inv,inv2" "42" "agents/inv/integration"
  meta_put "$parent" STATUS running
  meta_put "$parent" INVESTIGATION_LEASE 1

  local child_inv="20261102-000011-inv"
  local child_inv2="20261102-000012-inv2"
  meta_put "$parent" INVESTIGATION_RESUME_TASKS "$child_inv,$child_inv2"

  # Predeclared manifest: inv is running, inv2 is new (unstarted / uncreated)
  cat > "$AGENTS_HOME/tasks/$parent/resume.json" << EOF
{
  "version": 1,
  "members": [
    {"agent": "inv", "task": "$child_inv", "status": "running"},
    {"agent": "inv2", "task": "$child_inv2", "status": "new"}
  ]
}
EOF

  # child_inv metadata exists with status running. agent dispatch evaluates
  # cohort manifest recovery without exercising the periodic dead-runner sweep.
  mkdir -p "$AGENTS_HOME/tasks/$child_inv"
  cat > "$AGENTS_HOME/tasks/$child_inv/meta.env" << EOF
ID=$child_inv
AGENT=inv
KIND=investigation
INVESTIGATION_TEAM=$parent
INVESTIGATION_PHASE=5
REVIEW=1
STATUS=running
RUN=1
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF

  # child_inv2 directory does not exist (simulating crash during admission)

  run agent dispatch
  [ "$status" -eq 0 ]

  # Parent must retain lease and remain running because child_inv is still running
  [ "$(field "$parent" INVESTIGATION_LEASE)" = "1" ]
  [ "$(field "$parent" STATUS)" = "running" ]

  # Uncreated slot for inv2 must be failed in manifest snapshot
  [ "$(recorded_resume_status "$parent" inv2)" = "failed" ]
}

@test "after all resume reviewers reach terminal state handoff reports result and releases lease" {
  configure_investigators
  local parent="20261102-000020-investigation-team"
  setup_parent_resume "$parent" "inv,inv2" "42" "agents/inv/integration"
  meta_put "$parent" STATUS running
  meta_put "$parent" INVESTIGATION_LEASE 1

  local child_inv="20261102-000021-inv"
  local child_inv2="20261102-000022-inv2"
  meta_put "$parent" INVESTIGATION_RESUME_TASKS "$child_inv,$child_inv2"

  # Predeclared manifest with both failed
  cat > "$AGENTS_HOME/tasks/$parent/resume.json" << EOF
{
  "version": 1,
  "members": [
    {"agent": "inv", "task": "$child_inv", "status": "failed"},
    {"agent": "inv2", "task": "$child_inv2", "status": "failed"}
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
STATUS=failed
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
EOF

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

  # After all terminal, lease must be released and parent enters waiting
  [ "$(field "$parent" INVESTIGATION_LEASE)" != "1" ]
  [ "$(field "$parent" STATUS)" = "waiting" ]
  [[ "$(field "$parent" AUTO_STAGE)" == *"advisory reviews ready"* ]]
}
