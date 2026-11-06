#!/usr/bin/env bats
# Independent investigator resume prompt incorporates human notes and avoids duplicate answer sections.

load helpers

@test "independent investigator resume incorporates human note along with answered questions" {
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
  grep -q 'memory cache' "$HOME/independent-inv.prompt"
  grep -q 'Additional note from the human' "$HOME/independent-inv.prompt"
  grep -q 'Investigate only LRU cache eviction' "$HOME/independent-inv.prompt"
  agent dispatch > "$HOME/clarification-dispatch.log" 2>&1
  wait_status "$parent" 'done'
}

@test "independent investigator fresh session prompt does not duplicate answered questions" {
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

  # Set ANSWERS_PENDING to simulate pending answers — once the child's runner is done with its metadata, and under the
  # script's lock: a write of the runner's own replaced the file and lost an unlocked edit, under load
  local pid
  for _ in $(seq 1 150); do   # its runner removes runner.pid when its run ends
    pid=$(cat "$AGENTS_HOME/tasks/$child/runner.pid" 2> /dev/null) || break
    kill -0 "$pid" 2> /dev/null || break
    sleep 0.2
  done
  (source "$REPO_ROOT/bin/agent"; meta_set "$child" ANSWERS_PENDING 1)

  run bash -c "
    export AGENTS_HOME='$AGENTS_HOME'
    export TASKS='$AGENTS_HOME/tasks'
    export PROMPTS='$AGENTS_HOME/prompts'
    export WORKTREE='$HOME/repo'
    export FRESH_WHY='session timeout'
    source '$REPO_ROOT/bin/agent'
    meta_load '$child'
    build_prompt '$child' fresh '$qfile' 2
  "
  [ "$status" -eq 0 ]
  # Count occurrences of answered question text in rendered prompt
  # Count occurrences of resume answer header in rendered prompt
  local count
  count=$(grep -c "# THE HUMAN'S ANSWERS" <<< "$output")
  [ "$count" -eq 1 ]

  run agent resume "$child"
  [ "$status" -eq 0 ]
  wait_status "$child" 'done'
  agent dispatch > "$HOME/dispatch-teardown.log" 2>&1
  wait_status "$parent" 'done'
}
