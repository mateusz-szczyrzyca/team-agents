#!/usr/bin/env bats
# Independent investigators are shown under their own names, and the investigation hand-off is not called --autopilot.

load helpers

load investigation_pipeline_fixture

# labels_field parent key reads shell-quoted metadata from this test's temporary home.
labels_field() {
  bash -c 'source "$1"; printf "%s" "${!2-}"' bash "$AGENTS_HOME/tasks/$1/meta.env" "$2"
}

@test "agent status and show name independent investigators and the PR comment names --investigation" {
  single_pipeline_setup
  run agent run --investigation --team inv 'Investigate a temporary fixture'
  [ "$status" -eq 0 ]
  local parent phase1 merger
  parent=$(task_of investigation-team)
  wait_status "$parent" '^done$' 45
  phase1=$(labels_field "$parent" INVESTIGATION_PHASE1_TASKS)
  merger=$(labels_field "$parent" INVESTIGATION_MERGER_TASK)

  run agent status
  [ "$status" -eq 0 ]
  [[ $output != *'inv#'* ]]
  grep -E "^$phase1 +inv +done " <<< "$output"
  grep -E "^$merger +inv +done " <<< "$output"

  run agent show "$phase1"
  [ "$status" -eq 0 ]
  [[ $output == *"== investigation team $parent (phase 1)"* ]]
  [[ $output != *'investigation # for'* ]]
  run agent show "$merger"
  [ "$status" -eq 0 ]
  [[ $output == *"== investigation team $parent (phase 2)"* ]]

  grep -q -- '--body @example --investigation complete: review ready (#73)' "$HOME/gh.log"
  run grep -q -- '--autopilot' "$HOME/gh.log"
  [ "$status" -eq 1 ]
}
