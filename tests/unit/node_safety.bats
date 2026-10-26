#!/usr/bin/env bats
# Safety boundaries of the node daemon, using only temporary homes and stub CLIs.

load helpers

# safety_install copies the daemon under test; the override permits a RED run against a member's draft.
safety_install() {
  local script=${NODE_SAFETY_SCRIPT:-$REPO_ROOT/bin/agent}
  install -m755 "$script" "$HOME/.local/bin/agent"
}

@test "node state rejects malformed JSON and invalid shapes without replacing the file" {
  safety_install
  # shellcheck source=/dev/null
  source "$HOME/.local/bin/agent"
  declare -F node_jobs_load >/dev/null
  local data
  for data in '{broken' '[]' '"text"' '{"jobs":null,"outbox":[]}' \
    '{"jobs":[],"outbox":[]}' '{"jobs":{},"outbox":null}'; do
    printf '%s' "$data" > "$BUS_DIR/node-jobs.json"
    run node_jobs_load
    [ "$status" -ne 0 ]
    [ "$(cat "$BUS_DIR/node-jobs.json")" = "$data" ]
  done
}

@test "a non-file node state path is a storage failure rather than empty state" {
  safety_install
  # shellcheck source=/dev/null
  source "$HOME/.local/bin/agent"
  declare -F node_jobs_load >/dev/null
  mkdir "$BUS_DIR/node-jobs.json"
  run node_jobs_load
  [ "$status" -ne 0 ]
  [ -d "$BUS_DIR/node-jobs.json" ]
}

@test "the daemon lifetime lock rejects a second node process" {
  safety_install
  # shellcheck source=/dev/null
  source "$HOME/.local/bin/agent"
  declare -F node_lock >/dev/null
  node_lock
  run bash -c 'source "$1"; node_lock' _ "$HOME/.local/bin/agent"
  [ "$status" -ne 0 ]
  flock -u 8
  exec 8>&-
}

# safety_prepare closes Bats' descriptor 10 before the shared preparation path acquires its task lock.
safety_prepare() {
  (
    exec 10>&-
    prepare_job_task job-reserved solo repo main "" "prepare without launching" 1
    printf '%s' "$NEW_ID"
  )
}

@test "a prepared job reserves its agent while the node waits for the start acknowledgement" {
  safety_install
  # shellcheck source=/dev/null
  source "$HOME/.local/bin/agent"
  # shellcheck disable=SC2034 # prepare_job_task reads the map in the sourced daemon script.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local id busy
  id=$(safety_prepare)
  [ "$(task_field "$id" STATUS)" = new ]
  [ "$(task_field "$id" RUN)" = 0 ]
  [ "$(task_field "$id" JOB_ID)" = job-reserved ]
  busy=$(agent_running_task solo)
  [ "$busy" = "$id" ]
  run agent run solo "must wait for the prepared job"
  [ "$status" -ne 0 ]
  [ "$(find "$TASKS" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 1 ]
}
