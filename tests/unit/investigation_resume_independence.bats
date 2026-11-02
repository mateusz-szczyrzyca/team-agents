#!/usr/bin/env bats
# Answered clarification prompts preserve investigator isolation through resume.

load helpers

@test "answered investigator clarification resumes without requiring disabled agentbus tools" {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  scenario '{"inv":{"new":"questions"},"inv2":{"new":"publish"}}'
  run agent _investigation-team --team inv,inv2 'Investigate the cache'
  [ "$status" -eq 0 ]
  local parent child qfile prompt
  parent=$(task_of investigation-team)
  child=$(task_of inv)
  wait_status "$child" '^waiting$'
  qfile="$AGENTS_HOME/tasks/$child/questions-1.md"
  [ -s "$qfile" ]
  sed -i 's/^\*\*Verdict:\*\*$/\*\*Verdict:\*\* a)/' "$qfile"

  run agent resume "$child" 'Focus on eviction'
  [ "$status" -eq 0 ]
  wait_status "$child" '^done$'
  agent dispatch > "$HOME/independent-resume-dispatch.log" 2>&1
  wait_status "$parent" '^done$'

  prompt="$HOME/independent-inv.prompt"
  [ -s "$prompt" ]
  grep -q "# THE HUMAN'S ANSWERS" "$prompt"
  grep -q 'memory cache' "$prompt"
  grep -q 'Focus on eviction' "$prompt"
  grep -q 'STATUS: QUESTIONS' "$prompt"
  run grep -E '\{\{|read_messages|set_status' "$prompt"
  [ "$status" -eq 1 ]
}
