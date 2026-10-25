#!/usr/bin/env bats
# Additional review boundaries use real task metadata, file locks and temporary homes.

load helpers

# review_prepare agent job assignment returns an unstarted task without retaining Bats' descriptor 10.
review_prepare() {
  (
    exec 10>&-
    prepare_job_task "$2" "$1" repo main "" "review boundary" "$3"
    printf '%s' "$NEW_ID"
  )
}

# review_mapping job assignment [task] persists a recovery snapshot without starting any model.
review_mapping() {
  NODE_STATE=$(jq -nc --arg job "$1" --argjson assignment "$2" --arg tid "${3:-}" '
    {jobs:{($job):{assignment:$assignment,task_id:$tid,launched:false,start_acked:true,
      obsolete:false,pending:false,preparing:($tid == ""),state:"running",report:{State:"running"}}},outbox:[]}')
  node_jobs_save "$NODE_STATE"
}

@test "ambiguous unstarted tasks fail recovery without reserving either agent" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local first second
  first=$(review_prepare solo duplicate-job 3)
  second=$(review_prepare member duplicate-job 3)
  review_mapping duplicate-job 3
  ( exec 10>&-; node_recover_tasks )
  [ "$(jq -r '.jobs["duplicate-job"].state' "$(node_jobs_file)")" = failed ]
  run agent_running_task solo
  [ "$status" -eq 1 ]
  run agent_running_task member
  [ "$status" -eq 1 ]
  [ ! -e "$TASKS/$first/meta.env" ]
  [ ! -e "$TASKS/$second/meta.env" ]
}

@test "partial identity recovery preserves another assignment's task" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local old current
  old=$(review_prepare solo reused-job 1)
  current=$(review_prepare member reused-job 2)
  sed -i '/^JOB_ASSIGNMENT=/d' "$(meta_file "$current")"
  review_mapping reused-job 2
  ( exec 10>&-; node_recover_tasks )
  [ -f "$TASKS/$old/meta.env" ]
  [ "$(task_field "$old" JOB_ASSIGNMENT)" = 1 ]
  [ "$(task_field "$old" RUN)" = 0 ]
  [ "$(jq -r '.jobs["reused-job"].task_id' "$(node_jobs_file)")" = "$current" ]
  [ "$(task_field "$current" JOB_ASSIGNMENT)" = 2 ]
}

@test "active recovery preserves its reservation with stop markers and missing worktrees" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid missing
  tid=$(review_prepare solo active-marker 4)
  meta_set "$tid" RUN 1
  meta_set "$tid" STATUS running
  touch "$TASKS/$tid/stop-requested"
  # unit_active treats the simulated task/run as an active unit.
  unit_active() { return 0; }
  # start_runner records an unexpected task launch without invoking a model.
  start_runner() { printf '%s\n' "$1" > "$HOME/duplicate-launch"; }
  for missing in 0 1; do
    if ((missing)); then git -C "$HOME/repo" worktree remove --force "$(task_field "$tid" WORKTREE)"; fi
    review_mapping active-marker 4 "$tid"
    ( exec 10>&-; node_process_pending_launches; node_scan_tasks )
    [ ! -e "$HOME/duplicate-launch" ]
    [ "$(task_field "$tid" STATUS)" = running ]
    [ "$(jq -r '.jobs["active-marker"].state' "$(node_jobs_file)")" = running ]
    [ "$(agent_running_task solo)" = "$tid" ]
  done
}

@test "job stop waits for admission and releases admission before stopping its unit" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid child holder attempted=0 premature=0
  tid=$(review_prepare solo contended-stop 5)
  mkfifo "$HOME/release-lock"
  (
    exec 10>&-
    command flock "$AGENTS_HOME/task-ids.lock" bash -c \
      'printf "ready\n" > "$HOME/lock-ready"; read -r < "$HOME/release-lock"'
  ) &
  holder=$!
  wait_grep ready "$HOME/lock-ready"
  (
    exec 10>&-
    exec > "$HOME/stop-result"
    # This hook exposes the child's attempt, while the real flock still enforces contention.
    flock() {
      if [[ $* == '-w 60 10' ]]; then printf 'attempt\n' > "$HOME/stop-lock-attempt"; fi
      command flock "$@"
    }
    # systemctl probes admission release before forwarding the stub command arguments.
    systemctl() {
      if ! command flock -n "$AGENTS_HOME/task-ids.lock" true; then
        touch "$HOME/unit-stop-under-lock"
      fi
      command systemctl "$@"
    }
    cmd_stop "$tid"
  ) &
  child=$!
  # Always release the holder before assertions, including the expected RED missing-lock case.
  if wait_grep attempt "$HOME/stop-lock-attempt" 5; then attempted=1; fi
  if [[ -e $TASKS/$tid/stop-requested ]]; then premature=1; fi
  printf 'release\n' > "$HOME/release-lock"
  wait "$holder"
  wait "$child"
  [ "$attempted" -eq 1 ]
  [ "$premature" -eq 0 ]
  [ -f "$TASKS/$tid/stop-requested" ]
  [ "$(task_field "$tid" STATUS)" = stopped ]
  [ ! -e "$HOME/unit-stop-under-lock" ]
}

@test "recovery binds a unique started task instead of an unstarted duplicate" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local prepared started
  prepared=$(review_prepare lead started-duplicate 6)
  started=$(review_prepare solo started-duplicate 6)
  meta_set "$started" RUN 1
  meta_set "$started" STATUS running
  review_mapping started-duplicate 6
  ( exec 10>&-; node_recover_tasks )
  [ "$(jq -r '.jobs["started-duplicate"].task_id' "$(node_jobs_file)")" = "$started" ]
  [ "$(task_field "$started" STATUS)" = running ]
  [ "$(task_field "$started" RUN)" = 1 ]
  [ ! -e "$TASKS/$prepared/meta.env" ]
}

@test "multiple started matches stop recovery without destroying running tasks" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local first second tid
  first=$(review_prepare solo unsafe-duplicates 7)
  second=$(review_prepare member unsafe-duplicates 7)
  for tid in "$first" "$second"; do meta_set "$tid" RUN 1; meta_set "$tid" STATUS running; done
  review_mapping unsafe-duplicates 7
  # attempt_recovery closes Bats' inherited descriptor before the admission lock is acquired.
  attempt_recovery() { ( exec 10>&-; node_recover_tasks ); }
  run attempt_recovery
  [ "$status" -ne 0 ]
  for tid in "$first" "$second"; do
    [ "$(task_field "$tid" RUN)" = 1 ]
    [ "$(task_field "$tid" STATUS)" = running ]
  done
}

@test "a missing prepared worktree fails the local task and releases its agent" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # The sourced preparation function consumes this allow-list.
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  local tid
  tid=$(review_prepare solo missing-checkout 8)
  git -C "$HOME/repo" worktree remove --force "$(task_field "$tid" WORKTREE)"
  review_mapping missing-checkout 8 "$tid"
  ( exec 10>&-; node_process_pending_launches; node_scan_tasks )
  [ "$(jq -r '.jobs["missing-checkout"].state' "$(node_jobs_file)")" = failed ]
  [ "$(task_field "$tid" STATUS)" = failed ]
  run agent_running_task solo
  [ "$status" -eq 1 ]
}
