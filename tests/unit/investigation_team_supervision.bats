#!/usr/bin/env bats
# Tests that investigation team coordinator tasks (KIND=investigation_team) are not treated
# as dead runners by the supervisor, do not render with '?' in status, and that the supervisor
# drives dispatch to wake queued investigation teams when quota expires.

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

# launch_phase_one returns the newly allocated orchestration task through PHASE_ONE.
launch_phase_one() {
  run agent _investigation-team --team 'inv, inv2' -f "$HOME/topic.md"
  [ "$status" -eq 0 ]
  PHASE_ONE=$(task_of investigation-team)
  [ -n "$PHASE_ONE" ]
}

# recorded_status checks the immutable terminal snapshot of one phase-one member.
recorded_status() {
  jq -r --arg a "$2" '.members[] | select(.agent == $a) | .status' "$AGENTS_HOME/tasks/$1/phase1.json"
}

@test "cmd__supervise does not treat a running investigation_team coordinator task as a dead runner" {
  configure_investigators
  local id="20261101-000000-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
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
EOF
  # Age meta.env past AGENT_SUPERVISE_GRACE_SEC (120s)
  touch -d "5 minutes ago" "$dir/meta.env"

  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$id")" = "running" ]
}

@test "cmd_status does not append question mark to running investigation_team coordinator task" {
  configure_investigators
  local id="20261101-000000-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
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
EOF
  run agent status
  [ "$status" -eq 0 ]
  [[ $output != *"running?"* ]]
  [[ $output == *"running"* ]]
}

@test "cmd__supervise triggers dispatch to admit a queued cohort when quota expires" {
  independent_setup
  local id="20261101-000000-investigation-team"
  local dir="$AGENTS_HOME/tasks/$id"
  local base
  base=$(git -C "$HOME/repo" rev-parse HEAD)
  mkdir -p "$dir"
  cat > "$dir/meta.env" << EOF
ID=$id
AGENT=investigation-team
KIND=investigation_team
STATUS=new
AUTO_STAGE='waiting: capacity queue'
INVESTIGATION_PHASE=1
INVESTIGATION_TEAM=inv,inv2
INVESTIGATION_BASE_SHA=$base
INVESTIGATION_TOPIC_SLUG=topic
BASE=$base
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
EOF
  echo "test topic" > "$dir/task.md"

  # Create an old expired quota task for inv
  mkdir -p "$AGENTS_HOME/tasks/old-inv"
  cat > "$AGENTS_HOME/tasks/old-inv/meta.env" << EOF
AGENT=inv
KIND=investigation
STATUS=failed
QUOTA_UNTIL=$(( $(date +%s) - 60 ))
EOF

  scenario '{"inv":{"new":"publish"},"inv2":{"new":"publish"}}'

  run agent _supervise
  [ "$status" -eq 0 ]

  local c1 c2
  run task_of inv
  [ "$status" -eq 0 ]
  c1=$output
  run task_of inv2
  [ "$status" -eq 0 ]
  c2=$output
  [ -n "$c1" ]
  [ -n "$c2" ]
  wait_status "$c1" 'done'
  wait_status "$c2" 'done'
  wait_status "$id" waiting
  [ "$(recorded_status "$id" inv)" = 'done' ]
  [ "$(recorded_status "$id" inv2)" = 'done' ]
  [ -f "$AGENTS_HOME/tasks/old-inv/meta.env" ]
}

@test "an independent child hitting provider quota fails with QUOTA_UNTIL while peer succeeds and cohort waits" {
  independent_setup
  scenario '{"inv":{"new":"budget-spent"},"inv2":{"new":"publish"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  [ "$(status_of "$(task_of inv)")" = "failed" ]
  local quota
  quota=$(field "$(task_of inv)" QUOTA_UNTIL)
  [ -n "$quota" ]
  [ "$quota" -gt "$(date +%s)" ]
  [ "$(recorded_status "$PHASE_ONE" inv)" = "failed" ]
  [ "$(recorded_status "$PHASE_ONE" inv2)" = "done" ]
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" != 1 ]
  [ "$(field "$PHASE_ONE" AUTO_STAGE)" = 'waits for you: phase 1 complete (merge not available yet)' ]
}

