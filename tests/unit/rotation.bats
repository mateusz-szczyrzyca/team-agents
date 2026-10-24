#!/usr/bin/env bats
# Investigators rotate: the next request goes to the next free investigator after the last one, preferring another
# provider than the last one's — one provider is not used for every investigation.

load helpers

setup_rotation() {
  source "$REPO_ROOT/bin/agent"
  # three investigators: two at Anthropic, one at OpenAI (sorted: inv-a, inv-b, inv-c); read by the sourced script
  # shellcheck disable=SC2034
  declare -gA INVESTIGATORS_TYPE=([inv-a]=claude [inv-b]=claude [inv-c]=codex)
  # shellcheck disable=SC2034
  declare -gA INVESTIGATORS_MODEL=()
}

@test "the next investigator follows the last one, preferring another provider" {
  setup_rotation
  [ "$(pick_investigator "" inv-a inv-b inv-c)" = inv-a ]      # none yet: the first
  [ "$(pick_investigator inv-a inv-a inv-b inv-c)" = inv-c ]   # after Anthropic: OpenAI, not the next Anthropic one
  [ "$(pick_investigator inv-c inv-a inv-b inv-c)" = inv-a ]   # after OpenAI: the next one, at Anthropic
  [ "$(pick_investigator inv-a inv-a inv-b)" = inv-b ]         # only Anthropic ones free: rotate anyway
  [ "$(pick_investigator inv-b inv-a inv-b)" = inv-a ]
  run pick_investigator inv-a                                  # none free
  [ "$status" -ne 0 ]
}

@test "the last investigator is remembered, and the next free one is picked after it" {
  setup_rotation
  [ "$(free_investigator)" = inv-a ]
  remember_investigator inv-a
  [ "$(cat "$AGENTS_HOME/last-investigator")" = inv-a ]
  [ "$(free_investigator)" = inv-c ]
  remember_investigator inv-c
  [ "$(free_investigator)" = inv-a ]
}
