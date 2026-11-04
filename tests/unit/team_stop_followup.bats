#!/usr/bin/env bats
# Follow-up of PR #60: a stopped team keeps its finished members' results, an independent investigation defers a
# normal team's wakes instead of losing them, and the review pipeline of a team already at work is not dropped.
load helpers

# followup_fixture writes a two-member team (A coordinator, B member, both sleeping) without starting any process.
followup_fixture() {
  local id
  for id in A B; do
    mkdir -p "$TASKS/$id"
    printf 'ID=%s\nAGENT=%s\nKIND=worker\nTEAM=T-team\nRUN=1\nSTATUS=sleeping\nREPO_ROOT=%s\nREPO_NAME=repo\nWORKTREE=%s\n' \
      "$id" "$id" "$HOME/repo" "$HOME/repo" > "$TASKS/$id/meta.env"
  done
  printf '%s\n' '{"teams":{"T-team":{"id":"T-team","repo":"repo","lead":"A","members":{"A":{"state":"sleeping","tier":"normal","task":"A"},"B":{"state":"sleeping","tier":"normal","task":"B"}}}}}' > "$BUS_DIR/state.json"
}

# active_investigation writes a top-level --investigation of the fixture's repository with that status.
active_investigation() {
  mkdir -p "$TASKS/INV"
  printf 'ID=INV\nKIND=investigation_team\nSTATUS=%s\nREPO_ROOT=%s\nREPO_NAME=repo\n' "$1" "$HOME/repo" > "$TASKS/INV/meta.env"
}

# recording_self is a stand-in for the agent script that only records its arguments in $HOME/replayed.
recording_self() {
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "%s/replayed"\n' "$HOME" > "$HOME/self"
  chmod +x "$HOME/self"
  printf '%s' "$HOME/self"
}

@test "the automation check of a team task runs without a shell error" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  run automation_allowed A
  [ "$status" -eq 0 ]
  [[ $output != *"command not found"* ]]
}

@test "stopping a team keeps its finished members done, with their finishing time" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  meta_set B STATUS "done"
  meta_set B FINISHED 2026-11-04T10:00:00+01:00
  agentbus team-set --dir "$BUS_DIR" --team T-team --agent B --state "done"
  cmd_team T-team --stop
  [ "$(task_field A STATUS)" = stopped ]
  [ "$(task_field B STATUS)" = "done" ]
  [ "$(task_field B FINISHED)" = 2026-11-04T10:00:00+01:00 ]
  [ "$(agentbus team-get --dir "$BUS_DIR" --team T-team | jq -r '.members.B.state')" = "done" ]
  [ "$(agentbus team-get --dir "$BUS_DIR" --team T-team | jq -r '.members.A.state')" = stopped ]
}

@test "a member of a stopped team is not resumed with its answers, and the human is told why" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  meta_set B STATUS waiting
  meta_set B TEAM_STOPPED 1
  run "$SELF" resume B
  [ "$status" -ne 0 ]
  [[ $output == *"stopped"* ]]
  [ "$(task_field B RUN)" = 1 ]
}

@test "a member waiting for answers is not resumed while --investigation is in progress in its project" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  meta_set B STATUS waiting
  active_investigation running
  run "$SELF" resume B
  [ "$status" -ne 0 ]
  [[ $output == *"--investigation"* ]]
  [ "$(task_field B RUN)" = 1 ]
}

@test "a normal team's precheck still runs while --investigation is in progress in its project" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  active_investigation running
  team_wait_settled() { return 0; }
  review_precheck() { echo ran > "$HOME/precheck-ran"; return 1; }
  cmd__precheck A
  [ -f "$HOME/precheck-ran" ]
}

@test "a run that continues a team member (a bounce) is not refused while --investigation is in progress" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  active_investigation running
  systemd-run() { printf '%s\n' "$*" > "$HOME/launched"; }
  start_runner A resume
  [ -f "$HOME/launched" ]
  [ "$(task_field A RUN)" = 2 ]
}

@test "a wake held back by --investigation is recorded and replayed once the investigation ends" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  active_investigation waiting
  run "$SELF" _wake B
  [ "$status" -eq 0 ]
  [ "$(task_field B STATUS)" = sleeping ]
  [ "$(task_field B WAKE_DEFERRED)" = 1 ]
  local real=$SELF
  SELF=$(recording_self)
  supervise_deferred_wakes
  [ ! -e "$HOME/replayed" ]
  meta_set INV STATUS "done"
  supervise_deferred_wakes
  grep -qx '_wake B' "$HOME/replayed"
  SELF=$real
  run "$SELF" _wake B
  [ "$status" -eq 0 ]
  [ -z "$(task_field B WAKE_DEFERRED)" ]
  [ "$(task_field B RUN)" = 2 ]
}

@test "a finished --investigation still marked waiting does not block a normal team" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  followup_fixture
  active_investigation waiting
  meta_set INV INVESTIGATION_PIPELINE 1
  meta_set INV INVESTIGATION_PHASE 4
  meta_set INV INVESTIGATION_HANDOFF_RESULT failed
  run normal_resume_allowed A
  [ "$status" -eq 0 ]
}
