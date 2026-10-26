#!/usr/bin/env bats
# Node cancellation during runner finalization must keep the outcome and local notification.

load helpers

# cancel_exit_prepare builds a real task without launching a model or a systemd unit.
cancel_exit_prepare() {
  (
    exec 10>&-
    prepare_job_task exit-job solo repo main "" "cancel at finalization" 1
    printf '%s' "$NEW_ID"
  )
}

@test "node cancellation preserves each terminal outcome and its final result notification" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # prepare_job_task consumes this repository map.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid outcome finished='2026-10-25T20:00:00+00:00'
  tid=$(cancel_exit_prepare)
  meta_set "$tid" RUN 1
  touch "$TASKS/$tid/node-stop-requested"
  printf 'the actual final result\n' > "$TASKS/$tid/final.md"
  for outcome in "done" failed stopped hung excluded; do
    meta_set "$tid" STATUS "$outcome"
    meta_set "$tid" FINISHED "$finished"
    (
      # The signal can arrive between STATUS/FINISHED and RUNNER_DONE in the runner.
      meta_load "$tid"
      export RUNNER_ID=$tid RUNNER_DONE=0
      runner_exit 143
    )
    [ "$(task_field "$tid" STATUS)" = "$outcome" ]
    [ "$(task_field "$tid" FINISHED)" = "$finished" ]
    [ -f "$NOTIFY_DIR/$tid-r1-summary.md" ]
    grep -qF 'the actual final result' "$NOTIFY_DIR/$tid-r1-summary.md"
    grep -qF "\"status\":\"$outcome\"" "$LOG_DIR/agent.jsonl"
    rm "$NOTIFY_DIR/$tid-r1-summary.md"
  done
}

@test "node cancellation of an unfinished runner notifies locally as stopped" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # prepare_job_task consumes this repository map.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(cancel_exit_prepare)
  meta_set "$tid" RUN 1
  meta_set "$tid" STATUS running
  meta_set "$tid" FINISHED "2000-01-01T00:00:00+00:00"
  touch "$TASKS/$tid/node-stop-requested"
  (
    meta_load "$tid"
    export RUNNER_ID=$tid RUNNER_DONE=0
    runner_exit 143
  )
  [ "$(task_field "$tid" STATUS)" = stopped ]
  [ "$(task_field "$tid" FINISHED)" != "2000-01-01T00:00:00+00:00" ]
  [ -f "$NOTIFY_DIR/$tid-r1-summary.md" ]
  grep -qF 'Status: stopped' "$NOTIFY_DIR/$tid-r1-summary.md"
}

@test "node stop preserves every local terminal task status" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # prepare_job_task consumes this repository map.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid outcome
  tid=$(cancel_exit_prepare)
  export NODE_STATE
  NODE_STATE=$(jq -nc --arg tid "$tid" '{jobs:{"exit-job":{
    assignment:1,task_id:$tid,obsolete:false,launched:true,state:"running"
  }},outbox:[]}')
  for outcome in "done" failed stopped hung excluded; do
    meta_set "$tid" STATUS "$outcome"
    node_handle_stop exit-job 1
    [ "$(task_field "$tid" STATUS)" = "$outcome" ]
    [ ! -e "$TASKS/$tid/node-stop-requested" ]
  done
}

@test "node cancellation completes undelivered finalization after RUNNER_DONE" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # prepare_job_task consumes this repository map.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid delivered finished='2026-10-25T20:00:00+00:00'
  tid=$(cancel_exit_prepare)
  meta_set "$tid" RUN 1
  meta_set "$tid" STATUS "done"
  meta_set "$tid" FINISHED "$finished"
  touch "$TASKS/$tid/node-stop-requested"
  printf 'persisted completion\n' > "$TASKS/$tid/final.md"
  for delivered in 0 1; do
    (
      meta_load "$tid"
      export RUNNER_ID=$tid RUNNER_DONE=1 RUNNER_DELIVERED=$delivered
      runner_exit 143
    )
    [ "$(task_field "$tid" STATUS)" = "done" ]
    [ "$(task_field "$tid" FINISHED)" = "$finished" ]
    if [[ $delivered == 0 ]]; then
      grep -qF 'persisted completion' "$NOTIFY_DIR/$tid-r1-summary.md"
      rm "$NOTIFY_DIR/$tid-r1-summary.md"
    else
      [ ! -e "$NOTIFY_DIR/$tid-r1-summary.md" ]
    fi
  done
}
