#!/usr/bin/env bats
# A stop can arrive after start acknowledgement while local capacity still delays the actual launch.

load helpers

@test "a terminal prepared job is never launched after cancellation" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task reads this local repository allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(
    exec 10>&-
    prepare_job_task cancelled-job solo repo main "" "must never launch" 4
    printf '%s' "$NEW_ID"
  )
  meta_set "$tid" STATUS stopped
  NODE_STATE=$(jq -nc --arg tid "$tid" '{jobs:{"cancelled-job":{
    assignment:4,task_id:$tid,launched:false,start_acked:true,obsolete:false,
    pending:true,state:"stopped",report:{State:"stopped"}
  }},outbox:[]}')
  node_jobs_save "$NODE_STATE"

  # Capture an attempted launch without starting a real or stub model process.
  start_runner() { printf '%s\n' "$1" >> "$HOME/unexpected-node-launch"; }
  ( exec 10>&-; node_process_pending_launches )

  [ ! -e "$HOME/unexpected-node-launch" ]
  [ "$(task_field "$tid" RUN)" = 0 ]
  [ "$(task_field "$tid" STATUS)" = stopped ]
}

@test "failed job identity metadata prevents preparation and removes the unstarted checkout" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced prepare_job_task consumes the configured repository map.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local definition
  definition=$(declare -f meta_set)
  # The function body comes from the just-sourced repository script, never from task data.
  eval "${definition/meta_set/node_original_meta_set}"
  # meta_set injects failure only for the assignment marker; other writes keep the real implementation.
  meta_set() {
    [[ $2 != JOB_ASSIGNMENT ]] || return 1
    node_original_meta_set "$@"
  }
  # attempt_preparation reproduces the conditional caller without inheriting Bats' descriptor 10.
  attempt_preparation() {
    (
      exec 10>&-
      # The daemon calls preparation in a conditional; every required write must propagate its failure.
      if prepare_job_task metadata-failure solo repo main "" "never launch" 8; then return 0; else return 1; fi
    )
  }

  run attempt_preparation
  [ "$status" -ne 0 ]
  [ -z "$(find "$TASKS" -name meta.env -print)" ]
  [ -z "$(git -C "$HOME/repo" branch --list 'agents/*')" ]
}
