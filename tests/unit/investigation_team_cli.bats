#!/usr/bin/env bats
# Round 1 of investigation teams: `agent run --investigation` is parsed and every refusal is given before anything
# starts. A valid request is refused in this round as "not available yet" (the entry point stays inactive), and no
# task, branch or systemd unit is created in any case.

load helpers

# configure_investigators makes inv, inv2 and inv3 configured investigators in the temporary agents.conf.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' \
    "$AGENTS_HOME/agents.conf"
}

# no_side_effects asserts that a refused request left no task directory, no agent branch and no systemd unit.
no_side_effects() {
  [[ -z $(ls -A "$AGENTS_HOME/tasks") ]]
  [[ -z $(git -C "$HOME/repo" branch --list 'agents/*') ]]
  [[ -z $(ls -A "$STUB_UNITS") ]]
}

@test "--investigation without --team is refused before anything starts" {
  configure_investigators
  run agent run --investigation "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--team"* ]]
  no_side_effects
}

@test "--team with a single investigator is refused and the configured ones are listed" {
  configure_investigators
  run agent run --investigation --team "inv" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"two"* ]]
  [[ $output == *"inv2"* && $output == *"inv3"* ]]
  no_side_effects
}

@test "a duplicate investigator in --team is refused" {
  configure_investigators
  run agent run --investigation --team "inv, inv" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"duplicate"* ]]
  no_side_effects
}

@test "a name that is not a configured investigator is refused with the configured ones listed" {
  configure_investigators
  run agent run --investigation --team "inv, solo" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"solo"* ]]
  [[ $output == *"inv2"* && $output == *"inv3"* ]]
  no_side_effects
}

@test "an empty --team value is refused" {
  configure_investigators
  run agent run --investigation --team "" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"two"* ]]
  no_side_effects
}

@test "--team with investigators but without --investigation is refused" {
  configure_investigators
  run agent run --team "inv, inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--investigation"* ]]
  no_side_effects
}

@test "--investigation cannot be combined with --autopilot or --verify" {
  configure_investigators
  local flag
  for flag in --autopilot --verify; do
    run agent run --investigation --team "inv, inv2" "$flag" "topic"
    [ "$status" -ne 0 ]
    [[ $output == *"$flag"* ]]
  done
  no_side_effects
}

@test "--investigation cannot be combined with --need or --on (investigators run on the lead only)" {
  configure_investigators
  run agent run --investigation --team "inv, inv2" --need hw:rp2350 "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--need"* ]]
  run agent run --investigation --team "inv, inv2" --on node2 "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"--on"* ]]
  no_side_effects
}

@test "a valid two-investigator list with spaces after the commas is accepted but refused as not available yet" {
  configure_investigators
  run agent run --investigation --team "inv, inv2" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"not available yet"* ]]
  [[ $output != *"duplicate"* && $output != *"not a configured"* ]]
  no_side_effects
}

@test "three configured investigators are accepted and refused as not available yet" {
  configure_investigators
  run agent run --investigation --team "inv, inv2, inv3" "topic"
  [ "$status" -ne 0 ]
  [[ $output == *"not available yet"* ]]
  no_side_effects
}

@test "the topic can be given as a file with -f" {
  configure_investigators
  echo "topic from a file" > "$HOME/topic.md"
  run agent run --investigation --team "inv, inv2" -f "$HOME/topic.md"
  [ "$status" -ne 0 ]
  [[ $output == *"not available yet"* ]]
  no_side_effects
}

@test "a missing topic file given with -f is refused" {
  configure_investigators
  run agent run --investigation --team "inv, inv2" -f "$HOME/missing.md"
  [ "$status" -ne 0 ]
  [[ $output == *"missing.md"* ]]
  no_side_effects
}
