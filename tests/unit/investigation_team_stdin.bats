#!/usr/bin/env bats
# Tests public start of investigation teams with a piped topic from stdin.

load helpers

# configure_investigators makes inv, inv2 and inv3 configured investigators in the temporary agents.conf.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude [inv3]=claude)/' \
    "$AGENTS_HOME/agents.conf"
}

@test "piped topic from stdin starts an investigation team and preserves multiline content" {
  configure_investigators
  local expected=$'investigate race condition\nverify memory order\nreproduce on multiple cores'
  run agent run --investigation --team "inv, inv2" <<< "$expected"
  [ "$status" -eq 0 ]
  local parent
  parent=$(task_of investigation-team)
  [[ -n $parent ]]
  bash -c 'source "$1"; [[ $KIND == investigation_team && $INVESTIGATION_TEAM == "$2" && $INVESTIGATION_PIPELINE == 1 ]]' \
    bash "$AGENTS_HOME/tasks/$parent/meta.env" "inv,inv2"
  [[ $(cat "$AGENTS_HOME/tasks/$parent/task.md") == "$expected" ]]
}

@test "stdin stream read failure fails before creating investigation team parent task" {
  configure_investigators
  local test_bin="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$test_bin"
  cat > "$test_bin/cat" <<'CAT'
#!/usr/bin/env bash
echo "partial stream input"
exit 9
CAT
  chmod +x "$test_bin/cat"

  run env PATH="$test_bin:$PATH" agent run --investigation --team "inv, inv2" <<< "full stream payload"
  [ "$status" -ne 0 ]

  # task_of returns status 1 when no matching task exists; handle expected status 1 explicitly.
  run task_of investigation-team
  [ "$status" -eq 1 ]
  [[ -z $output ]]
  [[ -z $(ls -A "$AGENTS_HOME/tasks") ]]
}
