#!/usr/bin/env bats
# Merger and reviewer investigators may ask clarification questions without being replaced.

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

@test "a merger asks clarification questions and resumes without being replaced" {
  pipeline_setup
  scenario '{"inv": {"phase2": "questions"}, "inv2": {"phase2": "questions"}, "inv3": {"phase2": "questions"}}'
  launch_pipeline inv,inv2
  local merger merger_task qfile
  wait_grep 'INVESTIGATION_MERGER_TASK=20' "$AGENTS_HOME/tasks/$PIPELINE_PARENT/meta.env" 20
  merger_task=$(parent_field "$PIPELINE_PARENT" INVESTIGATION_MERGER_TASK)
  [ -n "$merger_task" ]
  merger=$(parent_field "$PIPELINE_PARENT" INVESTIGATION_MERGER)
  [ -n "$merger" ]
  wait_status "$merger_task" waiting 20
  [ "$(status_of "$PIPELINE_PARENT")" = running ]
  [ "$(parent_field "$PIPELINE_PARENT" INVESTIGATION_MERGER_REPLACEMENTS)" -eq 0 ]
  qfile=$(find "$AGENTS_HOME/tasks/$merger_task" -name 'questions-*.md' | head -n 1)
  [ -s "$qfile" ]
  grep -q 'Ambiguous investigation direction' "$qfile"
  sed -i 's/^\*\*Verdict:\*\*$/\*\*Verdict:\*\* a)/' "$qfile"
  run agent resume "$merger_task"
  [ "$status" -eq 0 ]
  wait_status "$merger_task" 'done' 20
  grep -q 'Focus on unit tests' "$HOME/phase2-$merger_task.prompt"
}

@test "a reviewer asks clarification questions and resumes without being replaced" {
  pipeline_setup
  scenario '{"inv": {"phase3": "questions"}, "inv2": {"phase3": "questions"}, "inv3": {"phase3": "questions"}}'
  launch_pipeline inv,inv2
  local reviewer reviewer_task qfile
  wait_grep 'INVESTIGATION_REVIEWER_TASK=20' "$AGENTS_HOME/tasks/$PIPELINE_PARENT/meta.env" 20
  reviewer_task=$(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEWER_TASK)
  [ -n "$reviewer_task" ]
  reviewer=$(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEWER)
  [ -n "$reviewer" ]
  wait_status "$reviewer_task" waiting 20
  [ "$(status_of "$PIPELINE_PARENT")" = running ]
  [ "$(parent_field "$PIPELINE_PARENT" INVESTIGATION_REVIEW_RETRIES)" -eq 0 ]
  qfile=$(find "$AGENTS_HOME/tasks/$reviewer_task" -name 'questions-*.md' | head -n 1)
  [ -s "$qfile" ]
  grep -q 'Ambiguous investigation direction' "$qfile"
  sed -i 's/^\*\*Verdict:\*\*$/\*\*Verdict:\*\* a)/' "$qfile"
  run agent resume "$reviewer_task"
  [ "$status" -eq 0 ]
  wait_status "$reviewer_task" 'done' 20
  grep -q 'Focus on unit tests' "$HOME/phase3-$reviewer_task.prompt"
}
