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

  # Set ANSWERS_PENDING to simulate pending answers
  sed -i 's/^ANSWERS_PENDING=.*/ANSWERS_PENDING=1/' "$AGENTS_HOME/tasks/$child/meta.env"
  if ! grep -q '^ANSWERS_PENDING=' "$AGENTS_HOME/tasks/$child/meta.env"; then
    echo "ANSWERS_PENDING=1" >> "$AGENTS_HOME/tasks/$child/meta.env"
  fi

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


@test "independent investigator fresh session prompt renders base prompt and topic only once and omits bus tools" {
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
  touch "$HOME/repo/AGENTS.md"

  run bash -c "
    export AGENTS_HOME='$AGENTS_HOME'
    export TASKS='$AGENTS_HOME/tasks'
    export PROMPTS='$AGENTS_HOME/prompts'
    export WORKTREE='$HOME/repo'
    export FRESH_WHY='session timeout'
    source '$REPO_ROOT/bin/agent'
    meta_load '$child'
    ANSWERS_PENDING=1
    build_prompt '$child' fresh '$qfile' 2
  "
  [ "$status" -eq 0 ]
  [ "$(grep -c '^# WORKING CONTEXT' <<< "$output")" -eq 1 ]
  [ "$(grep -c '^# TOPIC$' <<< "$output")" -eq 1 ]
  run ! grep -q 'CLARIFICATION RULES point 6' <<< "$output"
  run ! grep -q 'notebook_append' <<< "$output"

  run agent resume "$child"
  [ "$status" -eq 0 ]
  wait_status "$child" 'done'
  agent dispatch > "$HOME/dispatch-teardown.log" 2>&1
  wait_status "$parent" 'done'
}

@test "start_investigation review task delivers review request question and re-review context" {
  mkdir -p "$AGENTS_HOME/tasks/T1"
  printf 'ID=T1\nAGENT=lead\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=main\n' "$HOME/repo" "$HOME/repo" > "$AGENTS_HOME/tasks/T1/meta.env"
  printf 'Implement cache expiration.\n' > "$AGENTS_HOME/tasks/T1/task.md"
  run bash -c "
    source '$REPO_ROOT/bin/agent'
    bus() { :; }
    start_runner() { :; }
    j=\$(jq -n '{id:7, from_task:\"T1\", from_agent:\"lead\", kind:\"review\", pr_number:47, pr_url:\"https://github.com/o/r/pull/47\", question:\"Review PR #47 — one of two independent reviews.\n\nRe-review after fix pass 1: check cache expiration.\"}')
    start_investigation \"\$j\" inv
  "
  [ "$status" -eq 0 ]
  local inv_task
  inv_task=$(task_of inv7)
  grep -q 'one of two independent reviews' "$AGENTS_HOME/tasks/$inv_task/task.md"
  grep -q 'Re-review after fix pass 1: check cache expiration' "$AGENTS_HOME/tasks/$inv_task/task.md"
}

@test "helper prompt uses English Context heading and omits repository skipped-tests note" {
  printf '@test "p" {\n  skip "external hardware missing"\n}\n' > "$HOME/repo/skip.bats"
  git -C "$HOME/repo" add skip.bats
  git -C "$HOME/repo" -c user.name=t -c user.email=t@t commit -qm "add skip"
  mkdir -p "$AGENTS_HOME/tasks/T1"
  printf 'ID=T1\nAGENT=lead\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=main\n' "$HOME/repo" "$HOME/repo" > "$AGENTS_HOME/tasks/T1/meta.env"
  run bash -c "
    source '$REPO_ROOT/bin/agent'
    bus() { :; }
    start_runner() { :; }
    j=\$(jq -n --arg wt '$HOME/repo' '{id:3, from:\"lead\", from_task:\"T1\", repo_root:\$wt, repo:\"repo\", branch:\"main\", worktree:\$wt, request:\"Check files\", context:\"Only unit tests\"}')
    start_helper \"\$j\" worker
    htask=\$(ls '$AGENTS_HOME/tasks' | grep 'worker-h3')
    grep -q '^## Context$' \"$AGENTS_HOME/tasks/\$htask/task.md\"
    ! grep -q 'Kontekst' \"$AGENTS_HOME/tasks/\$htask/task.md\"
    [ -z \"\$(skip_note \"\$htask\")\" ]
  "
  [ "$status" -eq 0 ]
  grep -q 'request_investigation' "$REPO_ROOT/prompts/helper.md"
  grep -q 'checkpoint' "$REPO_ROOT/prompts/helper.md"
  run ! grep -q 'reserve' "$REPO_ROOT/prompts/review.md"
}
