#!/usr/bin/env bats
# Interrupted cohort admission: a leased parent whose admission never completed must not keep its investigator
# capacity forever. These tests simulate the half-written states a crash leaves behind.

load helpers

# configure_investigators sets up inv and inv2 as claude investigators in this test home only.
configure_investigators() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([inv]=claude [inv2]=claude)/' \
    "$AGENTS_HOME/agents.conf"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/claude"
  printf '%s\n' 'Independent topic' > "$HOME/topic.md"
}

# field decodes the shell-quoted metadata written by meta_set in this isolated test home.
field() { bash -c 'source "$1"; printf "%s" "${!2:-}"' _ "$AGENTS_HOME/tasks/$1/meta.env" "$2"; }

# meta_put sets KEY=value in a task's meta.env, replacing the line when the key already exists.
meta_put() {
  local file="$AGENTS_HOME/tasks/$1/meta.env"
  if grep -q "^$2=" "$file"; then sed -i "s|^$2=.*|$2=$3|" "$file"; else printf '%s=%s\n' "$2" "$3" >> "$file"; fi
}

# install_mv_probe puts a pass-through mv in this test home that records a parent whose lease is published
# before its phase1.json exists. meta_set writes every key through mv, and PATH finds this wrapper first.
install_mv_probe() {
  mkdir -p "$HOME/.local/bin"
  cat > "$HOME/.local/bin/mv" << 'PROBE'
#!/usr/bin/env bash
set -euo pipefail
# Pass-through mv: when a parent's meta.env gains INVESTIGATION_LEASE=1 without a manifest, record it.
src=${*: -2:1}
dst=${*: -1}
if [[ $dst == */*-investigation-team/meta.env ]] && grep -q '^INVESTIGATION_LEASE=1$' "$src" \
    && [[ ! -f ${dst%/meta.env}/phase1.json ]]; then
  printf '%s\n' "$dst" >> "$HOME/lease-before-manifest"
fi
exec /usr/bin/mv "$@"
PROBE
  chmod 755 "$HOME/.local/bin/mv"
}

# launch_phase_one returns the newly allocated orchestration task through PHASE_ONE.
launch_phase_one() {
  run agent _investigation-team --team 'inv, inv2' -f "$HOME/topic.md"
  [ "$status" -eq 0 ]
  PHASE_ONE=$(task_of investigation-team)
  [ -n "$PHASE_ONE" ]
}

# recorded_status reads the status of one member from the parent's phase1.json snapshot.
recorded_status() {
  jq -r --arg a "$2" '.members[] | select(.agent == $a) | .status' "$AGENTS_HOME/tasks/$1/phase1.json"
}

@test "a leased parent without phase1.json releases its lease when the supervisor runs" {
  configure_investigators
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"fail"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  # The crash window: admission took the lease and status, then died before writing the manifest.
  rm -f "$AGENTS_HOME/tasks/$PHASE_ONE/phase1.json"
  meta_put "$PHASE_ONE" INVESTIGATION_LEASE 1
  meta_put "$PHASE_ONE" STATUS running
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" != 1 ]
  [ "$(field "$PHASE_ONE" STATUS)" = waiting ]
}

@test "a legacy leased parent with neither manifest nor children releases its lease" {
  configure_investigators
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"fail"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  # Written before any child existed: no phase1.json and no child task is left to inspect.
  rm -f "$AGENTS_HOME/tasks/$PHASE_ONE/phase1.json"
  rm -rf "$AGENTS_HOME/tasks/$(task_of inv)" "$AGENTS_HOME/tasks/$(task_of inv2)"
  # The earliest crash happened before any child ID was recorded on the parent.
  meta_put "$PHASE_ONE" INVESTIGATION_PHASE1_TASKS ""
  meta_put "$PHASE_ONE" INVESTIGATION_LEASE 1
  meta_put "$PHASE_ONE" STATUS running
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" != 1 ]
  [ "$(field "$PHASE_ONE" STATUS)" = waiting ]
}

@test "a manifest member whose child metadata is missing is failed and releases the lease" {
  configure_investigators
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"fail"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  local child_inv2
  child_inv2=$(task_of inv2)
  # Interrupted before inv2's child task was written: the manifest names it, its metadata does not exist.
  rm -rf "$AGENTS_HOME/tasks/$child_inv2"
  # The manifest still lists inv2 as unfinished, so the tick must look at its missing metadata.
  local manifest="$AGENTS_HOME/tasks/$PHASE_ONE/phase1.json"
  jq '(.members[] | select(.agent == "inv2") | .status) = "new"' "$manifest" > "$manifest.tmp" && mv "$manifest.tmp" "$manifest"
  meta_put "$PHASE_ONE" INVESTIGATION_LEASE 1
  meta_put "$PHASE_ONE" STATUS running
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" != 1 ]
  [ "$(recorded_status "$PHASE_ONE" inv2)" = failed ]
}

@test "a child that never started (status new, no unit) counts as failed and releases the lease" {
  configure_investigators
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"fail"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  # Created by admission but never handed to start_runner: no unit, status still new.
  local child_inv2
  child_inv2=$(task_of inv2)
  meta_put "$child_inv2" STATUS new
  meta_put "$child_inv2" RUN 0
  # The manifest must still show inv2 as unfinished; the snapshot written after the run is not this state.
  local manifest="$AGENTS_HOME/tasks/$PHASE_ONE/phase1.json"
  jq '(.members[] | select(.agent == "inv2") | .status) = "new"' "$manifest" > "$manifest.tmp" && mv "$manifest.tmp" "$manifest"
  meta_put "$PHASE_ONE" INVESTIGATION_LEASE 1
  meta_put "$PHASE_ONE" STATUS running
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" != 1 ]
  [ "$(recorded_status "$PHASE_ONE" inv2)" = failed ]
}

@test "a genuinely running investigator keeps the cohort lease while the supervisor runs" {
  configure_investigators
  scenario '{"inv":{"new":"publish"},"inv2":{"new":"hold"}}'
  launch_phase_one
  wait_status "$(task_of inv)" 'done'
  # inv2 is still running under a live unit when the supervisor ticks: its lease must not be released.
  run agent _supervise
  [ "$status" -eq 0 ]
  [ "$(field "$PHASE_ONE" INVESTIGATION_LEASE)" = 1 ]
  touch "$HOME/release"
  wait_status "$PHASE_ONE" waiting
}

@test "the manifest is written before the parent takes its investigator lease" {
  configure_investigators
  install_mv_probe
  scenario '{"inv":{"new":"fail"},"inv2":{"new":"fail"}}'
  launch_phase_one
  wait_status "$PHASE_ONE" waiting
  # A lease published before phase1.json exists is exactly the window a crash turns into a leak.
  [ ! -e "$HOME/lease-before-manifest" ]
}

@test "a leased parent without manifest keeps the lease while its tagged child is still running" {
  configure_investigators
  scenario '{"inv":{"new":"publish"},"inv2":{"new":"hold"}}'
  launch_phase_one
  wait_status "$(task_of inv)" 'done'
  local live_child
  live_child=$(task_of inv2)
  # Crash window without a manifest: the still-running tagged child must keep the lease, not be ignored.
  rm -f "$AGENTS_HOME/tasks/$PHASE_ONE/phase1.json"
  meta_put "$PHASE_ONE" INVESTIGATION_PHASE1_TASKS ""
  meta_put "$PHASE_ONE" INVESTIGATION_LEASE 1
  meta_put "$PHASE_ONE" STATUS running
  run agent _supervise
  local supervise_status=$status lease_now child_now snapshot_now=no
  lease_now=$(field "$PHASE_ONE" INVESTIGATION_LEASE)
  child_now=$(field "$live_child" STATUS)
  # Recovery rebuilds the snapshot from the tagged children before it decides anything.
  [ -f "$AGENTS_HOME/tasks/$PHASE_ONE/phase1.json" ] && snapshot_now=yes
  # Release the live child before any assertion can fail, so the fixture finishes cleanly.
  touch "$HOME/release"
  wait_status "$live_child" 'done'
  [ "$supervise_status" -eq 0 ]
  [ "$lease_now" = 1 ]
  [ "$child_now" = running ]
  [ "$snapshot_now" = yes ]
}

@test "stop --running waits for admission before releasing investigation capacity" {
  configure_investigators
  scenario '{"inv":{"new":"hold"},"inv2":{"new":"hold"}}'
  mkfifo "$HOME/admission-release"
  # Pause real worktree creation after admission has published its lease and child IDs.
  cat > "$HOME/.local/bin/git" <<'GIT'
#!/usr/bin/env bash
set -euo pipefail
if [[ " $* " == *' worktree add '* && ! -f $HOME/admission-paused ]]; then
  printf 'ready\n' > "$HOME/admission-paused"
  read -r release < "$HOME/admission-release"
fi
exec /usr/bin/git "$@"
GIT
  # Signal when stop reaches its admission barrier. On unfixed code, completion signals instead.
  cat > "$HOME/.local/bin/flock" <<'FLOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ -f $AGENTS_HOME/.stop-all && /proc/self/fd/9 -ef $AGENTS_HOME/dispatch.lock ]]; then
  printf 'ready\n' > "$HOME/stop-observed"
fi
exec /usr/bin/flock "$@"
FLOCK
  chmod 755 "$HOME/.local/bin/git" "$HOME/.local/bin/flock"
  systemd-run --user --unit=admission-controller bash -c \
    'agent _investigation-team --team inv,inv2 -f "$HOME/topic.md" > "$HOME/admission.log" 2>&1; printf "done\n" > "$HOME/admission-ended"'
  wait_grep ready "$HOME/admission-paused"
  local parent
  parent=$(task_of investigation-team)
  [ "$(field "$parent" INVESTIGATION_LEASE)" = 1 ]
  [ ! -f "$AGENTS_HOME/tasks/$(jq -r '.members[0].task' "$AGENTS_HOME/tasks/$parent/phase1.json")/meta.env" ]
  systemd-run --user --unit=stop-controller bash -c \
    'agent stop --running > "$HOME/stop.log" 2>&1; printf "ready\n" > "$HOME/stop-observed"; printf "done\n" > "$HOME/stop-ended"'
  wait_grep ready "$HOME/stop-observed"
  printf 'continue\n' > "$HOME/admission-release"
  wait_grep "done" "$HOME/admission-ended"
  wait_grep "done" "$HOME/stop-ended"

  # Freeze the observed outcome before best-effort cleanup of the unfixed fixture's late runners.
  local lease first second first_state second_state first_active second_active available
  lease=$(field "$parent" INVESTIGATION_LEASE)
  first=$(task_of inv); second=$(task_of inv2)
  first_state=$(field "$first" STATUS); second_state=$(field "$second" STATUS)
  run systemctl --user is-active --quiet "agent-$first-r$(field "$first" RUN).service"
  first_active=$status
  run systemctl --user is-active --quiet "agent-$second-r$(field "$second" RUN).service"
  second_active=$status
  available=$(bash -c 'source "$1"; free_investigator_list' _ "$REPO_ROOT/bin/agent")
  run agent stop "$parent"
  [ "$status" -eq 0 ]
  [ "$(field "$parent" STATUS)" = stopped ]
  [ -z "$lease" ]
  [ "$first_state" = stopped ]
  [ "$second_state" = stopped ]
  [ "$first_active" -eq 3 ]
  [ "$second_active" -eq 3 ]
  [ "$available" = "$(printf 'inv\ninv2')" ]
}
