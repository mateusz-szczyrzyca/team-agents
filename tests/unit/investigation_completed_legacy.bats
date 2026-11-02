#!/usr/bin/env bats
# Completed handoffs from older installations must not wait forever after upgrade.

load helpers

# legacy_completion result creates the metadata written after an old Phase 4 handoff.
legacy_completion() {
  LEGACY_PARENT=20261101-000001-investigation-team
  mkdir -p "$AGENTS_HOME/tasks/$LEGACY_PARENT"
  cat > "$AGENTS_HOME/tasks/$LEGACY_PARENT/meta.env" <<META
ID=$LEGACY_PARENT
AGENT=investigation-team
KIND=investigation_team
TYPE=internal
STATUS=waiting
TEAM=
RUN=0
ROUND=0
REPO_NAME=repo
REPO_ROOT=$HOME/repo
WORKTREE=$HOME/repo
INVESTIGATION_PIPELINE=1
INVESTIGATION_PHASE=4
INVESTIGATION_LEASE=
INVESTIGATION_HANDOFF_PENDING=
INVESTIGATION_HANDOFF_STAGE=waits\\ for\\ you:\\ review\\ ready\\ \\(#73\\)
INVESTIGATION_HANDOFF_RESULT=$1
INVESTIGATION_COMMENTED=1
INVESTIGATION_NOTIFIED=1
PR_URL=https://github.com/example/repo/pull/73
PR_NUMBER=73
META
}

# legacy_field key reads the parent's shell-quoted metadata without inheriting caller values.
legacy_field() {
  bash -c 'source "$1"; printf "%s" "${!2-}"' bash "$AGENTS_HOME/tasks/$LEGACY_PARENT/meta.env" "$1"
}

# assert_no_legacy_delivery checks that recovery neither re-notifies nor starts an agent.
assert_no_legacy_delivery() {
  [ ! -e "$HOME/notifications.log" ]
  [ ! -e "$HOME/gh.log" ]
  [ "$(find "$AGENTS_HOME/tasks" -name meta.env | wc -l)" -eq 1 ]
  [ -z "$(ls -A "$STUB_UNITS")" ]
}

@test "supervisor finalizes an already delivered legacy success without another decision or notification" {
  legacy_completion 'done'
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$LEGACY_PARENT")" = 'done' ]
  [ "$(legacy_field AUTO_STAGE)" = 'complete: review ready (#73)' ]
  [ -z "$(legacy_field INVESTIGATION_HANDOFF_PENDING)" ]
  assert_no_legacy_delivery
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(status_of "$LEGACY_PARENT")" = 'done' ]
  assert_no_legacy_delivery
}

@test "dispatch finalizes an already delivered legacy failure with its recorded result" {
  legacy_completion failed
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(status_of "$LEGACY_PARENT")" = failed ]
  [ "$(legacy_field AUTO_STAGE)" = 'failed: review ready (#73)' ]
  assert_no_legacy_delivery
}

@test "legacy completion recovery does not guess a missing or invalid result" {
  local result
  for result in "" unknown; do
    legacy_completion "$result"
    run agent _supervise
    [ "$status" -eq 0 ]
    run agent dispatch
    [ "$status" -eq 0 ]
    [ "$(status_of "$LEGACY_PARENT")" = waiting ]
    assert_no_legacy_delivery
  done
}

@test "legacy completion recovery preserves unfinished phases and stopped parents" {
  local phase
  for phase in 1 2 3 5; do
    legacy_completion 'done'
    sed -i "s/^INVESTIGATION_PHASE=4$/INVESTIGATION_PHASE=$phase/" "$AGENTS_HOME/tasks/$LEGACY_PARENT/meta.env"
    run agent dispatch
    [ "$status" -eq 0 ]
    [ "$(status_of "$LEGACY_PARENT")" = waiting ]
    assert_no_legacy_delivery
  done
  legacy_completion 'done'
  sed -i 's/^STATUS=waiting$/STATUS=stopped/' "$AGENTS_HOME/tasks/$LEGACY_PARENT/meta.env"
  run agent _supervise
  [ "$status" -eq 0 ]
  run agent dispatch
  [ "$status" -eq 0 ]
  [ "$(status_of "$LEGACY_PARENT")" = stopped ]
  assert_no_legacy_delivery
}
