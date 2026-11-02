#!/usr/bin/env bats
# Public resume starts advisory reviews on the existing PR and never applies their suggestions.

load helpers

# advisory_field task key reads optional metadata without inheriting the test shell's values.
advisory_field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# advisory_parent state creates an old phase-four parent with no round-four fields and its PR fixture.
advisory_parent() {
  local state=$1 base
  parent=20261102-000000-investigation-team
  base=$(git rev-parse HEAD)
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-advisory-cli" "$HOME/.local/bin/claude"
  git branch agents/inv/original
  git push -q origin agents/inv/original
  git config url."$HOME/remote.git".insteadOf https://github.com/example/repo.git
  git remote set-url origin https://github.com/example/repo.git
  export PIPELINE_REAL_GIT
  PIPELINE_REAL_GIT=$(command -v git)
  mkdir -p "$HOME/pipeline-bin"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-git" "$HOME/pipeline-bin/git"
  export PATH="$HOME/pipeline-bin:$PATH"
  printf '\nRUN_PATH=%q\n' "$PATH" >> "$AGENTS_HOME/agents.conf"
  mkdir -p "$AGENTS_HOME/tasks/$parent"
  cat > "$AGENTS_HOME/tasks/$parent/meta.env" <<META
ID=$parent
AGENT=investigation-team
KIND=investigation_team
TYPE=internal
STATUS=waiting
REPO_ROOT=$HOME/repo
REPO_NAME=repo
WORKTREE=$HOME/repo
BRANCH=agents/inv/original
BASE=main
INVESTIGATION_BASE_SHA=$base
INVESTIGATION_TARGET_BRANCH=main
INVESTIGATION_PIPELINE=1
INVESTIGATION_PHASE=4
INVESTIGATION_TEAM=inv,inv2,inv3
INVESTIGATION_TOPIC_SLUG=advisory-topic
INVESTIGATION_MERGER=inv
INVESTIGATION_COMMENTED=1
INVESTIGATION_NOTIFIED=1
PRECHECK_HEAD=$base
PR_URL=https://github.com/example/repo/pull/73
PR_NUMBER=73
META
  printf '%s\n' 'Investigate caching' > "$AGENTS_HOME/tasks/$parent/task.md"
  jq -n --arg state "$state" --arg head "$base" \
    '[{number:73,url:"https://github.com/example/repo/pull/73",state:$state,headRefName:"agents/inv/original",headRefOid:$head,baseRefName:"main",body:""}]' > "$HOME/gh-prs.json"
  jq '.[0]' "$HOME/gh-prs.json" > "$HOME/gh-pr-view.json"
}

# assert_advisory_cohort checks every original member, including the merger, completed a review of the PR.
assert_advisory_cohort() {
  local child agent prompt
  [ "$(jq '.members | length' "$AGENTS_HOME/tasks/$parent/resume.json")" -eq 3 ]
  for agent in inv inv2 inv3; do
    child=$(jq -r --arg a "$agent" '.members[] | select(.agent==$a) | .task' "$AGENTS_HOME/tasks/$parent/resume.json")
    [ "$(advisory_field "$child" STATUS)" = 'done' ]
    [ "$(advisory_field "$child" REVIEW)" = 1 ]
    [ "$(advisory_field "$child" INVESTIGATION_TEAM)" = "$parent" ]
    [ "$(advisory_field "$child" PR_URL)" = https://github.com/example/repo/pull/73 ]
    prompt="$HOME/advisory-$child.prompt"
    [ -s "$prompt" ]
    grep -qi advisory "$prompt"
    grep -q 'VERDICT:' "$prompt"
    grep -q 'Investigate caching' "$prompt"
    git -C "$(advisory_field "$child" WORKTREE)" diff --quiet HEAD
  done
  [ -z "$(advisory_field "$parent" INVESTIGATION_LEASE)" ]
  [[ $(advisory_field "$parent" AUTO_STAGE) == *'complete: advisory reviews ready'* ]]
}

@test "open investigation PR resumes all original investigators including merger as advisory reviewers" {
  advisory_parent OPEN
  local head_before
  head_before=$(git rev-parse agents/inv/original)
  run agent run --resume "$parent" 'Check memory limits'
  [ "$status" -eq 0 ]
  wait_status "$parent" 'done'
  assert_advisory_cohort
  [ "$(git rev-parse agents/inv/original)" = "$head_before" ]
  grep -q 'Check memory limits' "$HOME"/advisory-*.prompt
  [ -s "$HOME/notifications.log" ]
  grep -q 'gh pr comment' "$HOME/gh.log"
  [ "$(jq -r '.[0].state' "$HOME/gh-prs.json")" = OPEN ]
  if grep -q 'gh pr merge' "$HOME/gh.log"; then
    echo 'advisory resume attempted a merge' >&2
    return 1
  fi
}

@test "merged investigation PR resume is refused with reason before creating tasks" {
  advisory_parent MERGED
  run agent run --resume "$parent"
  [ "$status" -ne 0 ]
  [[ $output == *merged* && $output == *'new topic'* ]]
  [ "$(find "$AGENTS_HOME/tasks" -name meta.env | wc -l)" -eq 1 ]
  [ ! -f "$HOME/advisory-calls.jsonl" ]
}

@test "closed investigation PR resume is refused with reason before creating tasks" {
  advisory_parent CLOSED
  run agent run --resume "$parent"
  [ "$status" -ne 0 ]
  [[ $output == *closed* && $output == *'new topic'* ]]
  [ "$(find "$AGENTS_HOME/tasks" -name meta.env | wc -l)" -eq 1 ]
  [ ! -f "$HOME/advisory-calls.jsonl" ]
}

@test "a completed investigation PR can receive another complete advisory pass" {
  advisory_parent OPEN
  run agent run --resume "$parent"
  [ "$status" -eq 0 ]
  wait_status "$parent" 'done'
  assert_advisory_cohort
  local previous
  previous=$(advisory_field "$parent" INVESTIGATION_RESUME_TASKS)
  run agent run --resume "$parent"
  [ "$status" -eq 0 ]
  wait_status "$parent" 'done'
  assert_advisory_cohort
  [ "$(advisory_field "$parent" INVESTIGATION_RESUME_TASKS)" != "$previous" ]
  [ "$(wc -l < "$HOME/advisory-calls.jsonl")" -eq 6 ]
  [ "$(advisory_field "$parent" INVESTIGATION_RESUME_COUNT)" -eq 2 ]
  [ "$(grep -c 'gh pr comment' "$HOME/gh.log")" -eq 2 ]
  [ "$(grep -c '^===' "$HOME/notifications.log")" -eq 2 ]
}
