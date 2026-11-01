#!/usr/bin/env bats
# A prompt carries a team's or task's newest checkpoints only (AGENT_PROMPT_CHECKPOINTS), and says where the earlier
# ones are: every checkpoint of a long task in every prompt was most of the prompt.

load helpers

@test "the prompt gets the newest checkpoints and where the earlier ones are" {
  mkdir -p "$AGENTS_HOME/bus/checkpoints"
  for i in 1 2 3 4; do
    printf '\n## Checkpoint 2026-11-01 0%s:00:00 — lead (branch b)\n\nstate number %s\n' "$i" "$i"
  done > "$AGENTS_HOME/bus/checkpoints/team-T-team.md"
  out=$(source "$REPO_ROOT/bin/agent"; AGENT_PROMPT_CHECKPOINTS=2 checkpoints_latest team-T-team)
  [[ $out == *"state number 3"* && $out == *"state number 4"* ]]
  [[ $out != *"state number 1"* && $out != *"state number 2"* ]]
  [[ $out == *"2 earlier checkpoints are in $AGENTS_HOME/bus/checkpoints/team-T-team.md"* ]]
}
