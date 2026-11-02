#!/usr/bin/env bats
# Independent investigators may clarify ambiguous topics without failing their cohort.

load helpers

@test "an independent investigator asks a question and resumes without failing its cohort" {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  scenario '{"inv":{"new":"questions"},"inv2":{"new":"publish"}}'
  run agent _investigation-team --team inv,inv2 'Investigate the cache'
  [ "$status" -eq 0 ]
  local parent child qfile
  parent=$(task_of investigation-team)
  child=$(task_of inv)
  wait_status "$child" waiting
  [ "$(status_of "$parent")" = running ]
  [ "$(jq -r '.members[] | select(.agent=="inv") | .status' "$AGENTS_HOME/tasks/$parent/phase1.json")" != failed ]
  wait_grep 'Status: ❓ waiting for answers' "$HOME/notifications.log"
  qfile=$(find "$AGENTS_HOME/tasks/$child" -name 'questions-*.md' | head -n 1)
  [ -s "$qfile" ]
  grep -q 'Which cache is in scope' "$qfile"
  sed -i 's/^\*\*Verdict:\*\*$/\*\*Verdict:\*\* a)/' "$qfile"
  run agent resume "$child"
  [ "$status" -eq 0 ]
  wait_status "$child" 'done'
  agent dispatch > "$HOME/clarification-dispatch.log" 2>&1
  [ "$(jq -r '.members[] | select(.agent=="inv") | .status' "$AGENTS_HOME/tasks/$parent/phase1.json")" = 'done' ]
  grep -q 'memory cache' "$HOME/independent-inv.prompt"
}
