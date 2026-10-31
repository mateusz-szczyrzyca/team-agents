#!/usr/bin/env bats
# Supplemental validation and boundary tests for Round 1 of investigation teams:
# `agent run --investigation` CLI parsing, strict refusal of invalid inputs and
# incompatibilities, whitespace handling, and assertion of no side effects.

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

@test "remote investigator specification is refused with explicit remote reason" {
  configure_investigators
  run agent run --investigation --team "inv, inv2@node2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"remote"* ]]
  no_side_effects
}

@test "--investigation cannot be combined with --resume" {
  configure_investigators
  run agent run --investigation --resume some-task-id --team "inv, inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--investigation"* ]]
  [[ $output == *"--resume"* ]]
  no_side_effects
}

@test "--investigation cannot be combined with --same-round" {
  configure_investigators
  run agent run --investigation --same-round --team "inv, inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--investigation"* ]]
  [[ $output == *"--same-round"* ]]
  no_side_effects
}

@test "--investigation cannot be combined with --last-team even with no prior team" {
  configure_investigators
  run agent run --investigation --last-team --team "inv, inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--investigation"* ]]
  [[ $output == *"--last-team"* ]]
  no_side_effects
}

@test "leading empty CSV component in --team is refused" {
  configure_investigators
  run agent run --investigation --team ",inv, inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"two"* || $output == *"empty"* ]]
  no_side_effects
}

@test "interior empty CSV component in --team is refused" {
  configure_investigators
  run agent run --investigation --team "inv,,inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"two"* || $output == *"empty"* ]]
  no_side_effects
}

@test "trailing empty CSV component in --team is refused" {
  configure_investigators
  run agent run --investigation --team "inv, inv2," "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"two"* || $output == *"empty"* ]]
  no_side_effects
}

@test "whitespace around names is trimmed and duplicate is detected" {
  configure_investigators
  run agent run --investigation --team "  inv  , inv " "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"duplicate"* ]]
  no_side_effects
}

@test "whitespace around valid distinct names is trimmed and accepted" {
  configure_investigators
  run agent run --investigation --team "  inv  ,  inv2  " "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"not available yet"* ]]
  [[ $output != *"duplicate"* && $output != *"unknown"* ]]
  no_side_effects
}

@test "reverse flag order --team before --investigation is accepted" {
  configure_investigators
  run agent run --team "inv, inv2" --investigation "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"not available yet"* ]]
  no_side_effects
}

@test "empty topic string argument is refused" {
  configure_investigators
  run agent run --investigation --team "inv, inv2" ""
  [ "$status" -ne 0 ]
  [[ $output == *"empty task"* || $output == *"give the task"* ]]
  no_side_effects
}

@test "blank-only topic string argument is refused" {
  configure_investigators
  run agent run --investigation --team "inv, inv2" "   "
  [ "$status" -ne 0 ]
  [[ $output == *"empty task"* ]]
  no_side_effects
}

@test "empty topic file given with -f is refused" {
  configure_investigators
  echo "" > "$HOME/empty-topic.md"
  run agent run --investigation --team "inv, inv2" -f "$HOME/empty-topic.md"
  [ "$status" -ne 0 ]
  [[ $output == *"empty task"* ]]
  no_side_effects
}
