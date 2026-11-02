#!/usr/bin/env bats
# A sole investigator must complete every pipeline phase without a human handoff.

load helpers

# single_pipeline_setup installs isolated phase-aware CLIs and a local GitHub remote substitute.
single_pipeline_setup() {
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
}

# single_parent_field parent key reads shell-quoted metadata from this test's temporary home.
single_parent_field() {
  bash -c 'source "$1"; printf "%s" "${!2-}"' bash "$AGENTS_HOME/tasks/$1/meta.env" "$2"
}

@test "sole investigator completes merge review and notification without a decision wait" {
  single_pipeline_setup
  run agent run --investigation --team inv 'Investigate a temporary fixture'
  [ "$status" -eq 0 ]
  local parent merger reviewer
  parent=$(task_of investigation-team)
  wait_status "$parent" '^(done|failed)$' 45
  [ "$(status_of "$parent")" = 'done' ]
  [ "$(single_parent_field "$parent" INVESTIGATION_PHASE)" = 4 ]
  [ "$(single_parent_field "$parent" INVESTIGATION_MERGER)" = inv ]
  [ "$(single_parent_field "$parent" INVESTIGATION_REVIEWER)" = inv ]
  merger=$(single_parent_field "$parent" INVESTIGATION_MERGER_TASK)
  reviewer=$(single_parent_field "$parent" INVESTIGATION_REVIEWER_TASK)
  [ -n "$reviewer" ]
  [ "$reviewer" != "$merger" ]
  [ "$(status_of "$reviewer")" = 'done' ]
  [ "$(jq -s '[.[] | select(.phase == "3")] | length' "$HOME/pipeline-calls.jsonl")" -eq 1 ]
  [ "$(single_parent_field "$parent" AUTO_STAGE)" = 'complete: review ready (#73)' ]
  [ -z "$(single_parent_field "$parent" INVESTIGATION_LEASE)" ]
  wait_grep 'Status: ✅ done' "$HOME/notifications.log"
  run grep -Ei 'Status: ❌ failed|waits for you|ready for your decision' "$HOME/notifications.log"
  [ "$status" -eq 1 ]
  run grep -q 'gh pr merge' "$HOME/gh.log"
  [ "$status" -eq 1 ]
}

@test "sole investigator review without a verdict still fails the completed flow" {
  single_pipeline_setup
  scenario '{"inv":{"phase3":"no-verdict"}}'
  run agent run --investigation --team inv 'Investigate a temporary fixture'
  [ "$status" -eq 0 ]
  local parent
  parent=$(task_of investigation-team)
  wait_status "$parent" '^failed$' 45
  [ "$(single_parent_field "$parent" AUTO_STAGE)" = 'failed: review failed' ]
  [ "$(jq -s '[.[] | select(.phase == "3")] | length' "$HOME/pipeline-calls.jsonl")" -eq 1 ]
  [ -z "$(single_parent_field "$parent" INVESTIGATION_LEASE)" ]
  grep -q 'Status: ❌ failed' "$HOME/notifications.log"
  run grep -Ei 'waits for you|ready for your decision' "$HOME/notifications.log"
  [ "$status" -eq 1 ]
}
