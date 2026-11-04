#!/usr/bin/env bats
# Persistent user stops and independent investigations must fence automatic resumption.
load helpers

# lifecycle_fixture builds metadata and a bus team without starting any processes.
lifecycle_fixture() {
  local id
  for id in A B; do
    mkdir -p "$TASKS/$id"
    printf 'ID=%s\nAGENT=%s\nKIND=worker\nTEAM=T-team\nRUN=1\nSTATUS=sleeping\nREPO_ROOT=%s\nREPO_NAME=repo\nWORKTREE=%s\n' \
      "$id" "$id" "$HOME/repo" "$HOME/repo" > "$TASKS/$id/meta.env"
  done
  printf '%s\n' '{"teams":{"T-team":{"id":"T-team","repo":"repo","lead":"A","members":{"A":{"state":"sleeping","tier":"normal","task":"A"},"B":{"state":"sleeping","tier":"normal","task":"B"}}}}}' > "$BUS_DIR/state.json"
}

@test "dashboard team stop persists every member and prevents wake" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  lifecycle_fixture
  cmd_team T-team --stop
  [ "$(task_field A STATUS)" = stopped ]
  [ "$(task_field B STATUS)" = stopped ]
  [ -f "$TASKS/A/stop-requested" ]
  [ -f "$TASKS/B/stop-requested" ]
  [ "$(team_done_flag T-team)" = true ]
  run "$SELF" _wake B
  [ "$status" -eq 0 ]
  [ "$(task_field B STATUS)" = stopped ]
  [ "$(task_field B RUN)" = 1 ]
}

@test "dashboard stopped automation cannot launch a fix pass or review" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  lifecycle_fixture
  touch "$TASKS/A/stop-requested"
  review_precheck() { echo unexpected > "$HOME/automation-ran"; }
  do_fix_pass() { echo unexpected > "$HOME/automation-ran"; }
  team_wait_settled() { return 0; }
  request_review() { return 0; }
  dispatch() { return 0; }
  printf '1' > "$TASKS/A/fix-pass.n"
  printf 'finding' > "$TASKS/A/fix-pass.md"
  cmd__precheck A
  cmd__fixpass A
  [ ! -e "$HOME/automation-ran" ]
}

@test "dashboard stopped runner exits without restarting team automation" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  lifecycle_fixture
  touch "$TASKS/A/stop-requested"
  meta_load A
  export RUNNER_ID=A RUNNER_DONE=0
  dispatch() { return 0; }
  dispatch_helpers() { return 0; }
  team_after_run() { echo unexpected > "$HOME/automation-ran"; }
  runner_exit 143
  [ "$(task_field A STATUS)" = stopped ]
  [ ! -e "$HOME/automation-ran" ]
}

@test "dashboard independent investigation blocks normal resume before mutation" {
  export AGENT_INVESTIGATION_EXCLUSIVE=yes   # an --investigation holds its project only when it is to
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  lifecycle_fixture
  mkdir -p "$TASKS/INV"
  printf 'ID=INV\nKIND=investigation_team\nSTATUS=running\nREPO_ROOT=%s\n' "$HOME/repo" > "$TASKS/INV/meta.env"
  run "$SELF" run --resume T-team --same-round "continue"
  [ "$status" -ne 0 ]
  [[ $output == *"independent investigation INV"* ]]
  [ "$(task_field A STATUS)" = sleeping ]
  [ "$(find "$TASKS" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 3 ]
}

@test "dashboard independent investigation does not hold back a message wake" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  lifecycle_fixture
  mkdir -p "$TASKS/INV"
  printf 'ID=INV\nKIND=investigation_team\nSTATUS=waiting\nREPO_ROOT=%s\n' "$HOME/repo" > "$TASKS/INV/meta.env"
  run "$SELF" _wake B
  [ "$status" -eq 0 ]
  [ -z "$(task_field B WAKE_DEFERRED)" ]
  [ "$(task_field B RUN)" = 2 ]
}

@test "dashboard ordinary investigations and other projects do not block resume" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  lifecycle_fixture
  mkdir -p "$TASKS/INV"
  printf 'ID=INV\nKIND=investigation\nSTATUS=running\nREPO_ROOT=%s\n' "$HOME/repo" > "$TASKS/INV/meta.env"
  run "$SELF" run --resume T-team --same-round "continue"
  [[ $output == *'is still active'* ]]
  meta_set INV KIND investigation_team
  meta_set INV REPO_ROOT "$HOME/other-repo"
  run "$SELF" run --resume T-team --same-round "continue"
  [[ $output == *'is still active'* ]]
}
