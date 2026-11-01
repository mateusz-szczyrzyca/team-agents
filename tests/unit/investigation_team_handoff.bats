#!/usr/bin/env bats
# End-to-end investigation pipeline regressions use isolated homes and fake investigator CLIs.

load helpers

# pipeline_setup installs phase-aware fixtures and enables three local investigators.
pipeline_setup() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' \
    "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/independent-fixture"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-pipeline-cli" "$HOME/.local/bin/claude"
  printf '\nAGENT_REVIEW_GATE=true\nAGENT_STAGE_POLL_SEC=1\n' >> "$AGENTS_HOME/agents.conf"
  git config url."$HOME/remote.git".insteadOf https://github.com/example/repo.git
  git remote set-url origin https://github.com/example/repo.git
  export PIPELINE_REAL_GIT
  PIPELINE_REAL_GIT=$(command -v git)
  mkdir -p "$HOME/pipeline-bin"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-git" "$HOME/pipeline-bin/git"
  export PATH="$HOME/pipeline-bin:$PATH"
  printf '\nRUN_PATH=%q\n' "$PATH" >> "$AGENTS_HOME/agents.conf"
  export AGENT_INVESTIGATION_SEED=7
}

# parent_field parent key reads one field without inheriting another task's metadata.
parent_field() {
  bash -c 'source "$1"; printf "%s" "${!2-}"' bash "$AGENTS_HOME/tasks/$1/meta.env" "$2"
}

# launch_pipeline starts the full internal pipeline with the given investigator list.
launch_pipeline() {
  if ! agent _investigation-team --pipeline --team "$1" "Investigate a temporary fixture" > "$HOME/pipeline-start.log" 2>&1; then
    cat "$HOME/pipeline-start.log"
    return 1
  fi
  PIPELINE_PARENT=$(task_of investigation-team)
}

@test "a completed pipeline opens a PR and uses the unchanged review prompt" {
  pipeline_setup
  launch_pipeline inv,inv2
  wait_status "$PIPELINE_PARENT" '^waiting$' 45
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_PHASE) == 4 ]]
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_PR) == https://github.com/example/repo/pull/73 ]]
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEWER) != "$(parent_field "$PIPELINE_PARENT" INVESTIGATION_MERGER)" ]]
  grep -q 'gh pr create --base main --head agents/' "$HOME/gh.log"
  local reviewer
  reviewer=$(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEWER_TASK)
  grep -q 'WORKING CONTEXT — PR REVIEW (investigator)' "$HOME/phase3-$reviewer.prompt"
  grep -q 'Review by the team-agents investigator' "$HOME/phase3-$reviewer.prompt"
  run grep -q '{{' "$HOME/phase3-$reviewer.prompt"
  [ "$status" -eq 1 ]
  [[ $(jq -s '[.[] | select(.phase == "2")] | length' "$HOME/pipeline-calls.jsonl") == 1 ]]
}

@test "handoff records the stage comments mentions the owner and never merges" {
  pipeline_setup
  launch_pipeline inv,inv2
  wait_status "$PIPELINE_PARENT" '^waiting$' 45
  [[ $(parent_field "$PIPELINE_PARENT" AUTO_STAGE) == 'waits for you: review ready (#73)' ]]
  [[ -z $(parent_field "$PIPELINE_PARENT" INVESTIGATION_LEASE) ]]
  grep -q 'gh pr comment https://github.com/example/repo/pull/73' "$HOME/gh.log"
  grep -q " --body @example " "$HOME/gh.log"
  grep -q "$PIPELINE_PARENT" "$HOME/notifications.log"
  run grep -q 'gh pr merge' "$HOME/gh.log"
  [ "$status" -eq 1 ]
  agent dispatch > "$HOME/repeated-dispatch.log" 2>&1
  [[ $(grep -c 'gh pr comment' "$HOME/gh.log") == 1 ]]
}

@test "failed checks bounce once then replace the merger once and wait" {
  pipeline_setup
  printf '\nAGENT_REVIEW_GATE=false\n' >> "$AGENTS_HOME/agents.conf"
  launch_pipeline inv,inv2
  wait_status "$PIPELINE_PARENT" '^waiting$' 60
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_MERGE_RETRIES) == 1 ]]
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_MERGER_REPLACEMENTS) == 1 ]]
  [[ $(parent_field "$PIPELINE_PARENT" AUTO_STAGE) == 'waits for you: merge failed' ]]
  [[ $(jq -s '[.[] | select(.phase == "2")] | length' "$HOME/pipeline-calls.jsonl") == 3 ]]
  [[ $(jq -s '[.[] | select(.phase == "3")] | length' "$HOME/pipeline-calls.jsonl") == 0 ]]
  run grep -q 'gh pr merge' "$HOME/gh.log"
  [ "$status" -eq 1 ]
}

@test "a review without a verdict is replaced only once before human handoff" {
  pipeline_setup
  scenario '{"inv":{"phase3":"no-verdict"},"inv2":{"phase3":"no-verdict"},"inv3":{"phase3":"no-verdict"}}'
  launch_pipeline inv,inv2,inv3
  wait_status "$PIPELINE_PARENT" '^waiting$' 60
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEW_RETRIES) == 1 ]]
  [[ $(parent_field "$PIPELINE_PARENT" AUTO_STAGE) == 'waits for you: review failed' ]]
  [[ $(jq -s '[.[] | select(.phase == "3")] | length' "$HOME/pipeline-calls.jsonl") == 2 ]]
  [[ $(jq -s '[.[] | select(.phase == "3")] | map(.agent) | unique | length' "$HOME/pipeline-calls.jsonl") == 2 ]]
  run grep -q 'gh pr merge' "$HOME/gh.log"
  [ "$status" -eq 1 ]
}

@test "findings without an explicit verdict still fail the investigation review" {
  pipeline_setup
  scenario '{"inv":{"phase3":"findings-no-verdict"},"inv2":{"phase3":"findings-no-verdict"},"inv3":{"phase3":"findings-no-verdict"}}'
  launch_pipeline inv,inv2,inv3
  wait_status "$PIPELINE_PARENT" '^waiting$' 60
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEW_RETRIES) == 1 ]]
  [[ $(parent_field "$PIPELINE_PARENT" AUTO_STAGE) == 'waits for you: review failed' ]]
  [[ $(jq -s '[.[] | select(.phase == "3")] | length' "$HOME/pipeline-calls.jsonl") == 2 ]]
}

@test "supervision completes an interrupted phase four handoff only once" {
  pipeline_setup
  local parent=20261101-000001-investigation-team
  mkdir -p "$AGENTS_HOME/tasks/$parent"
  cat > "$AGENTS_HOME/tasks/$parent/meta.env" <<META
ID=$parent
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
INVESTIGATION_HANDOFF_RESULT=done
PR_URL=https://github.com/example/repo/pull/73
PR_NUMBER=73
META
  run agent _supervise
  [ "$status" -eq 0 ]
  [[ -z $(parent_field "$parent" INVESTIGATION_HANDOFF_PENDING) ]]
  grep -q 'gh pr comment https://github.com/example/repo/pull/73' "$HOME/gh.log"
  grep -q "$parent" "$HOME/notifications.log"
  agent dispatch > "$HOME/repeated-dispatch.log" 2>&1
  [[ $(grep -c 'gh pr comment' "$HOME/gh.log") == 1 ]]
}

@test "a pipeline with no successful phase one result goes directly to human decision" {
  pipeline_setup
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"fail"}}'
  launch_pipeline inv,inv2
  wait_status "$PIPELINE_PARENT" '^waiting$' 45
  [[ $(parent_field "$PIPELINE_PARENT" INVESTIGATION_PHASE) == 4 ]]
  [[ $(parent_field "$PIPELINE_PARENT" AUTO_STAGE) == 'waits for you: all phase 1 runs failed' ]]
  [[ -z $(parent_field "$PIPELINE_PARENT" INVESTIGATION_LEASE) ]]
  [[ -z $(parent_field "$PIPELINE_PARENT" PR_URL) ]]
  grep -q "$PIPELINE_PARENT" "$HOME/notifications.log"
  [[ ! -e $HOME/pipeline-calls.jsonl ]]
  if [[ -f $HOME/gh.log ]]; then
    run grep -Eq 'gh pr (create|merge|comment)' "$HOME/gh.log"
    [ "$status" -eq 1 ]
  fi
}
