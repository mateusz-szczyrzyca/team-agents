#!/usr/bin/env bats
# Tests for allowing a single investigator in investigation teams.

load helpers

# configure_investigators configures inv, inv2 and inv3 in temporary agents.conf.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' \
    "$AGENTS_HOME/agents.conf"
}

# no_side_effects asserts that no task, branch, stub unit, or worktree was created.
no_side_effects() {
  [[ -z $(ls -A "$AGENTS_HOME/tasks") ]]
  [[ -z $(git -C "$HOME/repo" branch --list 'agents/*') ]]
  [[ -z $(ls -A "$STUB_UNITS") ]]
  if [[ -d "$AGENTS_HOME/worktrees" ]]; then
    [[ -z $(ls -A "$AGENTS_HOME/worktrees") ]]
  fi
}

@test "single investigator is accepted and investigation-team task is created" {
  configure_investigators
  run agent run --investigation --team "inv" "single investigator task"
  [ "$status" -eq 0 ]
  local parent
  parent=$(task_of investigation-team)
  [[ -n $parent ]]
  bash -c 'source "$1"; [[ $KIND == investigation_team && $INVESTIGATION_TEAM == "$2" && $INVESTIGATION_PIPELINE == 1 ]]' \
    bash "$AGENTS_HOME/tasks/$parent/meta.env" "inv"
  [[ $(cat "$AGENTS_HOME/tasks/$parent/task.md") == "single investigator task" ]]
}

@test "single investigator with whitespace is trimmed and accepted" {
  configure_investigators
  run agent run --investigation --team "  inv  " "topic"
  [ "$status" -eq 0 ]
  local parent
  parent=$(task_of investigation-team)
  [[ -n $parent ]]
  bash -c 'source "$1"; [[ $KIND == investigation_team && $INVESTIGATION_TEAM == "$2" ]]' \
    bash "$AGENTS_HOME/tasks/$parent/meta.env" "inv"
}

