#!/usr/bin/env bats
# Regressions for review findings R1 and R4:
# R1: Dead independent runner supervision must not crash on unbound FROM_AGENT
#     and must not skip cohort dispatch.
# R4: Phase-one completion must use auto_stage no-comment and not inherit caller PR_URL.

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

@test "dead independent runner supervision marks child failed and runs dispatch without crashing" {
  independent_setup
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

  run env AGENT_SUPERVISE_GRACE_SEC=0 agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$child1" STATUS)" = "failed" ]
  [ "$(recorded_status "$parent" inv)" = "failed" ]
  [ "$(recorded_status "$parent" inv2)" = "failed" ]
  [ "$(field "$parent" STATUS)" = "waiting" ]
  [ "$(field "$parent" AUTO_STAGE)" = 'waits for you: all phase 1 runs failed' ]
}

@test "phase one completion does not inherit caller PR_URL or comment on unrelated PR" {
  independent_setup
  export PR_URL="https://github.com/example/unrelated/pull/999"
  scenario '{"inv":{"new":"publish"},"inv2":{"new":"publish"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  [ -f "$STUB_HOME/gh.log" ] || touch "$STUB_HOME/gh.log"
  run grep "pr comment" "$STUB_HOME/gh.log"
  [ "$status" -eq 1 ]
}
