#!/usr/bin/env bats
# What the agents write on GitHub about themselves and the repository: an investigator's attribution names its CLI
# and model (it is configured in INVESTIGATORS_TYPE, not AGENT_TYPE); the rule for file links covers deleted files,
# directories and paths that need encoding.

load helpers

@test "an investigator's attribution names its CLI and model, not (?)" {
  local out
  out=$( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2034  # both read by the sourced script's agent_desc
    INVESTIGATORS_TYPE["inv-x"]=codex INVESTIGATORS_MODEL["inv-x"]=gpt-6-astra
    AGENT=inv-x TEAM='' attribution_block '' )
  [[ $out == *'- agent: `inv-x` (codex, model `gpt-6-astra`)'* ]]
  [[ $out != *'(?)'* ]]
}

@test "a team agent's attribution still comes from AGENT_TYPE" {
  local out
  out=$( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2034  # both read by the sourced script's agent_desc
    AGENT_TYPE["codex-1"]=codex AGENT_MODEL["codex-1"]=gpt-6.1-sol
    agent_desc codex-1 )
  [ "$out" = '`codex-1` (codex, model `gpt-6.1-sol`)' ]
}

@test "the file-link rule: deleted files at the base commit, directories as tree, encoded paths, no invented links" {
  local w=$REPO_ROOT/prompts/writing.md
  grep -q 'deleted' "$w"
  grep -q '/tree/<full SHA>/' "$w"
  grep -qi 'URL-encode' "$w"
  grep -qi 'code block' "$w"
}
