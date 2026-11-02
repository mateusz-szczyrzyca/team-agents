#!/usr/bin/env bats
# Independent investigator resume prompt omits contradictory agentbus instructions.

load helpers

@test "independent investigator resume prompt omits agentbus tools and coordination instructions" {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  scenario '{"inv":{"new":"questions"},"inv2":{"new":"publish"}}'
  run agent _investigation-team --team inv,inv2 'Investigate the cache'
  [ "$status" -eq 0 ]
  local parent child qfile
  parent=$(task_of investigation-team)
  child=$(task_of inv)
  wait_status "$child" waiting
  qfile=$(find "$AGENTS_HOME/tasks/$child" -name 'questions-*.md' | head -n 1)
  [ -s "$qfile" ]
  sed -i 's/^\*\*Verdict:\*\*$/\*\*Verdict:\*\* a)/' "$qfile"

  run agent resume "$child" "Investigate only LRU cache eviction"
  [ "$status" -eq 0 ]
  wait_status "$child" 'done'
  [ -f "$HOME/independent-inv.prompt" ]
  grep -q 'Additional note from the human' "$HOME/independent-inv.prompt"
  grep -q 'Investigate only LRU cache eviction' "$HOME/independent-inv.prompt"
  grep -q 'strict independence' "$HOME/independent-inv.prompt"
  grep -q 'no agentbus MCP tools' "$HOME/independent-inv.prompt"
  run ! grep -q 'read_messages' "$HOME/independent-inv.prompt"
  run ! grep -q 'set_status' "$HOME/independent-inv.prompt"
  run ! grep -q 'teammates' "$HOME/independent-inv.prompt"

  agent dispatch > "$HOME/clarification-dispatch.log" 2>&1
  wait_status "$parent" 'done'
}
