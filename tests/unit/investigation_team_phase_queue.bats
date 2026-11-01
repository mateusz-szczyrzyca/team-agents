#!/usr/bin/env bats
# Tests for investigation team phase 2 and 3 capacity queueing, quota expiration
# wake-up via supervisor, and dead precheck recovery.

load helpers

# field task_id key decodes the shell-quoted metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# configure_investigators sets up inv and inv2 in agents.conf.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' \
    "$AGENTS_HOME/agents.conf"
}

# independent_setup configures two investigators and a fake CLI in this test's temporary home.
independent_setup() {
  configure_investigators
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  printf '%s\n' 'Independent topic' > "$HOME/topic.md"
}

# create_parent_task id team base phase sets up an investigation_team coordinator task with standard fields.
create_parent_task() {
  local id=$1 team=$2 base=$3 phase=$4
  local dir="$AGENTS_HOME/tasks/$id"
  # commit-tree fixtures need an identity independent of the caller's environment.
  git -C "$HOME/repo" config user.name 'Test'
  git -C "$HOME/repo" config user.email 'test@example.com'
  mkdir -p "$dir"
  cat > "$dir/meta.env" << EOF
ID=$id
AGENT=investigation-team
KIND=investigation_team
STATUS=running
TYPE=internal
MODEL=
EXTRA_ARGS=
TEAM=
TIER=normal
SESSION_ID=
RUN=0
ROUND=0
RUNNER_DONE=1
CREATED=$(date -Is)
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BASE=$base
INVESTIGATION_PIPELINE=1
INVESTIGATION_PHASE=$phase
INVESTIGATION_TEAM=$team
INVESTIGATION_BASE_SHA=$base
INVESTIGATION_TOPIC_SLUG=topic
INVESTIGATION_LEASE=1
AUTO_STAGE='waiting: capacity queue'
EOF
}

@test "cmd__supervise triggers dispatch to resume phase 2 when quota expires" {
  independent_setup
  local id="20261101-000020-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  mkdir -p "$AGENTS_HOME/tasks/old-inv" "$AGENTS_HOME/tasks/old-inv2"
  cat > "$AGENTS_HOME/tasks/old-inv/meta.env" << EOF
AGENT=inv
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) - 60 ))
EOF
  cat > "$AGENTS_HOME/tasks/old-inv2/meta.env" << EOF
AGENT=inv2
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) - 60 ))
EOF

  scenario '{"inv":{"new":"hold"},"inv2":{"new":"hold"}}'

  run agent _supervise
  [ "$status" -eq 0 ]

  [ -n "$(field "$id" INVESTIGATION_MERGER)" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"merging: "* ]]
}

@test "cmd__supervise leaves phase 2 queued while quota remains active" {
  independent_setup
  local id="20261101-000021-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 2

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  mkdir -p "$AGENTS_HOME/tasks/old-inv" "$AGENTS_HOME/tasks/old-inv2"
  cat > "$AGENTS_HOME/tasks/old-inv/meta.env" << EOF
AGENT=inv
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) + 3600 ))
EOF
  cat > "$AGENTS_HOME/tasks/old-inv2/meta.env" << EOF
AGENT=inv2
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) + 3600 ))
EOF

  run agent _supervise
  [ "$status" -eq 0 ]

  [ -z "$(field "$id" INVESTIGATION_MERGER)" ]
  [ "$(field "$id" AUTO_STAGE)" = 'waiting: capacity queue' ]
}

@test "cmd__supervise triggers dispatch to resume phase 3 reviewer when quota expires" {
  independent_setup
  local id="20261101-000022-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  create_parent_task "$id" 'inv,inv2' "$base" 3

  cat >> "$dir/meta.env" << EOF
INVESTIGATION_MERGER=inv
PR_NUMBER=42
PR_URL=https://github.com/example/repo/pull/42
EOF

  local c1 c2
  c1=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c1" "$base^{tree}")
  c2=$(git -C "$HOME/repo" commit-tree -p "$base" -m "c2" "$base^{tree}")
  cat > "$dir/phase1.json" << EOF
{"version":1,"members":[
  {"agent":"inv","task":"task-inv","status":"done","commit":"$c1"},
  {"agent":"inv2","task":"task-inv2","status":"done","commit":"$c2"}
]}
EOF

  mkdir -p "$AGENTS_HOME/tasks/old-inv2"
  cat > "$AGENTS_HOME/tasks/old-inv2/meta.env" << EOF
AGENT=inv2
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) - 60 ))
EOF

  scenario '{"inv2":{"new":"hold"}}'

  run agent _supervise
  [ "$status" -eq 0 ]

  [ "$(field "$id" INVESTIGATION_REVIEWER)" = "inv2" ]
  [[ "$(field "$id" AUTO_STAGE)" == *"review: inv2 (#42)"* ]]
}

@test "dispatch recovers from dead precheck unit and bounces to merger with retry count 1" {
  independent_setup
  local id="20261101-000023-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base child
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  child="20261101-000023-child"
  create_parent_task "$id" 'inv,inv2' "$base" 2
  mkdir -p "$AGENTS_HOME/tasks/$child"

  cat >> "$dir/meta.env" << EOF
INVESTIGATION_MERGER=inv
INVESTIGATION_MERGER_TASK=$child
INVESTIGATION_PRECHECK=running
INVESTIGATION_PRECHECK_UNIT=agent-$id-precheck-1
INVESTIGATION_MERGE_RETRIES=0
PR_NUMBER=42
PR_URL=https://github.com/example/repo/pull/42
BRANCH=agents/inv/topic
EOF

  git -C "$HOME/repo" worktree add -q -b agents/inv/topic "$HOME/merger-worktree" "$base"
  cat > "$AGENTS_HOME/tasks/$child/meta.env" << EOF
ID=$child
AGENT=inv
KIND=investigation
TYPE=claude
MODEL=
EXTRA_ARGS=
TEAM=
STATUS=done
RUN=0
ROUND=0
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
BASE=$base
BRANCH=agents/inv/topic
INVESTIGATION_TEAM=$id
INVESTIGATION_PHASE=2
EOF

  printf 'Merger findings\n' > "$AGENTS_HOME/tasks/$child/final.md"
  printf 'Merger task instructions\n' > "$AGENTS_HOME/tasks/$child/task.md"

  scenario '{"inv":{"new":"hold"}}'

  # First dispatch: detects inactive precheck unit and marks INVESTIGATION_PRECHECK failed
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_PRECHECK)" = "failed" ]

  # Second dispatch: executes the bounce, incrementing retries to 1 and restarting child
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(field "$id" INVESTIGATION_MERGE_RETRIES)" = "1" ]
  [ "$(field "$child" RUN)" = "1" ]
  grep -q '# Checks to fix' "$AGENTS_HOME/tasks/$child/task.md"
  grep -q "Checks to fix" "$AGENTS_HOME/tasks/$child/task.md"
  [ "$(field "$child" RUN)" = "1" ]
  # Wait for the resumed runner and parent handoff before teardown removes their logs.
  wait_status "$id" waiting
}
