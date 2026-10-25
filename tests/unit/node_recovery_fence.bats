#!/usr/bin/env bats
# Regressions for launch fencing, recovery of inactive tasks, and partial job metadata.

load helpers

@test "recovery marks inactive RUN=1 STATUS=new task as failed" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task stranded-job solo repo main "" "task body" 1
    printf '%s' "$NEW_ID"
  )
  # Simulate crash where RUN was incremented to 1 before runner started or status became starting
  meta_set "$tid" RUN 1
  meta_set "$tid" STATUS new
  NODE_STATE=$(jq -nc --arg tid "$tid" '{jobs:{"stranded-job":{
    assignment:1,task_id:$tid,launched:false,start_acked:true,obsolete:false,
    pending:false,state:"running",report:{State:"running"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  start_runner() { printf '%s\n' "$1" >> "$HOME/unexpected-launch"; }
  unit_active() { return 1; }

  ( exec 10>&-; node_process_pending_launches; node_scan_tasks )

  [ ! -e "$HOME/unexpected-launch" ]
  [ "$(task_field "$tid" STATUS)" = failed ]
  local state
  state=$(jq -r '.jobs["stranded-job"].state' <<< "$(node_jobs_load)")
  [ "$state" = failed ]
}

@test "final launch fence prevents start_runner when task was stopped during start ACK delay" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task delayed-ack-job solo repo main "" "task body" 2
    printf '%s' "$NEW_ID"
  )
  NODE_STATE=$(jq -nc --arg tid "$tid" '{jobs:{"delayed-ack-job":{
    assignment:2,task_id:$tid,launched:false,start_acked:false,obsolete:false,
    pending:false,state:"running",report:{State:"running"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  # node_start_ack simulates a delayed ACK where stop arrives while waiting for ACK
  node_start_ack() {
    local jid=$1
    touch "$TASKS/$tid/stop-requested"
    meta_set "$tid" STATUS stopped
    NODE_STATE=$(jq -c --arg id "$jid" '.jobs[$id].start_acked=true' <<< "$NODE_STATE")
    node_save
  }

  start_runner() { printf '%s\n' "$1" >> "$HOME/unexpected-launch"; }

  ( exec 10>&-; node_process_pending_launches )

  [ ! -e "$HOME/unexpected-launch" ]
  [ "$(task_field "$tid" RUN)" = 0 ]
}

@test "crash between separate JOB_* writes recovers partial task and does not orphan agent reservation" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task interrupted-job solo repo main "" "task body" 5
    printf '%s' "$NEW_ID"
  )
  # Simulate crash after JOB_ID write but before JOB_ASSIGNMENT write:
  local f
  f=$(meta_file "$tid")
  sed -i '/^JOB_ASSIGNMENT=/d' "$f"

  NODE_STATE=$(jq -nc '{jobs:{"interrupted-job":{
    assignment:5,task_id:"",launched:false,preparing:true,obsolete:false,
    pending:false,state:"running",report:{State:"running"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  ( exec 10>&-; node_recover_tasks )

  local recovered_tid
  recovered_tid=$(jq -r '.jobs["interrupted-job"].task_id // empty' <<< "$(node_jobs_load)")
  [ -n "$recovered_tid" ]
  [ "$recovered_tid" = "$tid" ]
  [ "$(task_field "$tid" JOB_ASSIGNMENT)" = "5" ]
}

@test "cmd_stop synchronizes with task ID lock to fence pending launches" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task stopped-early solo repo main "" "task body" 6
    printf '%s' "$NEW_ID"
  )
  NODE_STATE=$(jq -nc --arg tid "$tid" '{jobs:{"stopped-early":{
    assignment:6,task_id:$tid,launched:false,start_acked:true,obsolete:false,
    pending:false,state:"running",report:{State:"running"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  # Stop marker written while status is still new
  touch "$TASKS/$tid/stop-requested"

  start_runner() { printf '%s\n' "$1" >> "$HOME/unexpected-launch"; }

  ( exec 10>&-; node_process_pending_launches )

  [ ! -e "$HOME/unexpected-launch" ]
  [ "$(task_field "$tid" RUN)" = 0 ]
}

@test "recovery completes missing JOB_LEAD on matched task" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task matched-lead-job solo repo main "" "task body" 8
    printf '%s' "$NEW_ID"
  )
  # Simulate crash after JOB_ASSIGNMENT but before JOB_LEAD:
  local f
  f=$(meta_file "$tid")
  sed -i '/^JOB_LEAD=/d' "$f"

  # shellcheck disable=SC2034 # LEAD_URL is read by node_recover_tasks.
  declare -g LEAD_URL="http://lead:7780"
  NODE_STATE=$(jq -nc '{jobs:{"matched-lead-job":{
    assignment:8,task_id:"",launched:false,preparing:true,obsolete:false,
    pending:false,state:"running",report:{State:"running"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  ( exec 10>&-; node_recover_tasks )

  local recovered_tid
  recovered_tid=$(jq -r '.jobs["matched-lead-job"].task_id // empty' <<< "$(node_jobs_load)")
  [ "$recovered_tid" = "$tid" ]
  [ "$(task_field "$tid" JOB_LEAD)" = "http://lead:7780" ]
}

@test "stop marker persists stopped on unstarted task so reports transition to stopped" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task marker-only-job solo repo main "" "task body" 9
    printf '%s' "$NEW_ID"
  )
  NODE_STATE=$(jq -nc --arg tid "$tid" '{jobs:{"marker-only-job":{
    assignment:9,task_id:$tid,launched:false,start_acked:true,obsolete:false,
    pending:false,state:"running",report:{State:"running"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  # Stop marker without status update
  touch "$TASKS/$tid/stop-requested"

  start_runner() { printf '%s\n' "$1" >> "$HOME/unexpected-launch"; }

  ( exec 10>&-; node_process_pending_launches; node_scan_tasks )

  [ ! -e "$HOME/unexpected-launch" ]
  [ "$(task_field "$tid" STATUS)" = stopped ]
  local state
  state=$(jq -r '.jobs["marker-only-job"].state' <<< "$(node_jobs_load)")
  [ "$state" = stopped ]
}

@test "cmd_stop locks task IDs before writing stop-requested and releases before unit stop" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task stop-sync solo repo main "" "task body" 10
    printf '%s' "$NEW_ID"
  )
  local -a trace=()
  lock_task_ids() { trace+=("lock"); }
  unlock_task_ids() { trace+=("unlock"); }
  touch() {
    if [[ ${1:-} == */stop-requested ]]; then trace+=("marker"); fi
    command touch "$@"
  }
  unit_stop_failed() { :; }
  systemctl() { trace+=("systemctl"); return 0; }

  ( exec 10>&-; cmd_stop "$tid" )

  # Must lock first, then write marker, then unlock before systemctl stop
  [ "${trace[0]:-}" = "lock" ]
  [ "${trace[1]:-}" = "marker" ]
  [ "${trace[2]:-}" = "unlock" ]
  [ "${trace[3]:-}" = "systemctl" ]
}
