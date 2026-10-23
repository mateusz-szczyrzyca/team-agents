#!/usr/bin/env bats

load helpers

# install_date_shim creates a local date shim returning MOCK_TIMESTAMP for +%Y%m%d-%H%M%S.
install_date_shim() {
  cat > "$HOME/.local/bin/date" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "+%Y%m%d-%H%M%S" && $# -eq 1 && -n "${MOCK_TIMESTAMP:-}" ]]; then
  echo "$MOCK_TIMESTAMP"
  exit 0
fi
exec /bin/date "$@"
EOF
  chmod +x "$HOME/.local/bin/date"
}

# wait_task_runner_exit waits for every recorded runner to exit before metadata snapshots.
wait_task_runner_exit() {
  local t=$1 max=${2:-10} pid_file pid stat state i found=0 exited
  for pid_file in "$STUB_UNITS"/agent-"$t"-*.service.pid; do
    [[ -f $pid_file ]] || continue
    found=1
    pid=$(<"$pid_file")
    exited=0
    for ((i = 0; i < max * 10; i++)); do
      if [[ ! -d /proc/$pid ]]; then exited=1; break; fi
      if IFS= read -r stat < "/proc/$pid/stat"; then
        # A zombie has finished all writes even if its parent has not reaped it.
        state=${stat##*) }
        state=${state%% *}
        if [[ $state == Z ]]; then exited=1; break; fi
      elif [[ ! -d /proc/$pid ]]; then
        # The runner may exit between the existence check and reading stat.
        exited=1
        break
      else
        echo "cannot read runner PID $pid for task $t" >&2
        return 1
      fi
      sleep 0.1
    done
    if ((exited == 0)); then
      echo "runner PID $pid for task $t did not exit within $max seconds" >&2
      return 1
    fi
  done
  if ((found == 0)); then
    echo "missing runner PID file for task $t under $STUB_UNITS" >&2
    return 1
  fi
}

# teardown waits for isolated stub groups so failed assertions cannot race temporary-home removal.
teardown() {
  local pid_file pid task
  for pid_file in "$STUB_UNITS"/*.service.pid; do
    [[ -f $pid_file ]] || continue
    pid=$(<"$pid_file")
    if [[ -d /proc/$pid ]]; then
      if ! kill -TERM -- "-$pid"; then
        # A runner may disappear after the existence check; other failures matter.
        [[ ! -d /proc/$pid ]] || return 1
      fi
    fi
  done
  for pid_file in "$STUB_UNITS"/*.service.pid; do
    [[ -f $pid_file ]] || continue
    task=${pid_file##*/}
    task=${task#agent-}
    task=${task%-r*.service.pid}
    wait_task_runner_exit "$task" || return 1
  done
}

@test "same-second team resume allocates fresh IDs and preserves predecessor task metadata" {
  install_date_shim
  export MOCK_TIMESTAMP="20261023-120000"

  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team chapter 1"
  old_lead=$(task_of lead)
  old_member=$(task_of member)
  wait_status "$old_lead" running
  wait_status "$old_member" running
  agent stop --running
  wait_status "$old_lead" stopped
  wait_status "$old_member" stopped
  wait_task_runner_exit "$old_lead"
  wait_task_runner_exit "$old_member"

  team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$old_lead/meta.env")
  old_lead_branch=$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$old_lead/meta.env")
  old_member_branch=$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$old_member/meta.env")

  # Add a questions file in the predecessor to verify history is preserved
  echo "decision: proceed with plan" > "$AGENTS_HOME/tasks/$old_lead/questions-1.md"

  # Snapshot predecessors' task.md and meta.env byte-for-byte after runners exited
  cp "$AGENTS_HOME/tasks/$old_lead/task.md" "$HOME/old_lead_task.md"
  cp "$AGENTS_HOME/tasks/$old_lead/meta.env" "$HOME/old_lead_meta.env"
  cp "$AGENTS_HOME/tasks/$old_member/task.md" "$HOME/old_member_task.md"
  cp "$AGENTS_HOME/tasks/$old_member/meta.env" "$HOME/old_member_meta.env"

  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  run timeout --kill-after=2s 15s agent run --resume "$team" "team chapter 2 addition"
  [ "$status" -eq 0 ]

  new_lead=$(task_of lead)
  new_member=$(task_of member)
  new_team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$new_lead/meta.env")

  # Fresh IDs must be allocated; team ID must differ from old team
  [ "$new_lead" != "$old_lead" ]
  [ "$new_member" != "$old_member" ]
  [ "$new_team" != "$team" ]

  # Check four distinct task directories exist
  [ -d "$AGENTS_HOME/tasks/$old_lead" ]
  [ -d "$AGENTS_HOME/tasks/$old_member" ]
  [ -d "$AGENTS_HOME/tasks/$new_lead" ]
  [ -d "$AGENTS_HOME/tasks/$new_member" ]

  # Predecessor task.md and meta.env must be byte-for-byte unchanged
  cmp -s "$AGENTS_HOME/tasks/$old_lead/task.md" "$HOME/old_lead_task.md"
  cmp -s "$AGENTS_HOME/tasks/$old_lead/meta.env" "$HOME/old_lead_meta.env"
  cmp -s "$AGENTS_HOME/tasks/$old_member/task.md" "$HOME/old_member_task.md"
  cmp -s "$AGENTS_HOME/tasks/$old_member/meta.env" "$HOME/old_member_meta.env"

  # Predecessors must not have self-referential RESUMED_FROM
  [ -z "$(sed -n 's/^RESUMED_FROM=//p' "$AGENTS_HOME/tasks/$old_lead/meta.env")" ]
  [ -z "$(sed -n 's/^RESUMED_FROM=//p' "$AGENTS_HOME/tasks/$old_member/meta.env")" ]

  # New tasks link correctly to their exact predecessor
  [ "$(sed -n 's/^RESUMED_FROM=//p' "$AGENTS_HOME/tasks/$new_lead/meta.env")" = "$old_lead" ]
  [ "$(sed -n 's/^RESUMED_FROM=//p' "$AGENTS_HOME/tasks/$new_member/meta.env")" = "$old_member" ]

  # Both branches must remain unchanged
  [ "$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$new_lead/meta.env")" = "$old_lead_branch" ]
  [ "$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$new_member/meta.env")" = "$old_member_branch" ]

  # New task.md contains original text and addition
  grep -q "team chapter 1" "$AGENTS_HOME/tasks/$new_lead/task.md"
  grep -q "team chapter 2 addition" "$AGENTS_HOME/tasks/$new_lead/task.md"

  # History from predecessor questions is preserved in history.md
  grep -q "decision: proceed with plan" "$AGENTS_HOME/tasks/$new_lead/history.md"

  # Stub runs complete
  wait_status "$new_lead" 'done'
  wait_status "$new_member" 'done'
}

@test "team resume advances past taskdir, worktree, branch, and team registry collisions" {
  install_date_shim
  export MOCK_TIMESTAMP="20261023-140000"

  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "first task"
  old_lead=$(task_of lead)
  old_member=$(task_of member)
  wait_status "$old_lead" running
  wait_status "$old_member" running
  agent stop --running
  wait_status "$old_lead" stopped
  wait_status "$old_member" stopped
  wait_task_runner_exit "$old_lead"
  wait_task_runner_exit "$old_member"
  team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$old_lead/meta.env")

  # 1) Pre-create an unrelated conflicting task directory at candidate + 1 second (140001)
  mkdir -p "$AGENTS_HOME/tasks/20261023-140001-lead"
  echo "taskdir data" > "$AGENTS_HOME/tasks/20261023-140001-lead/sentinel.txt"

  # 2) Pre-create an unrelated conflicting worktree directory at candidate + 2 seconds (140002)
  mkdir -p "$AGENTS_HOME/worktrees/repo/20261023-140002-lead"
  echo "worktree data" > "$AGENTS_HOME/worktrees/repo/20261023-140002-lead/sentinel.txt"

  # 3) Pre-create an unrelated conflicting git branch at candidate + 3 seconds (140003)
  git -C "$HOME/repo" branch "agents/lead/20261023-140003"

  # 4) Pre-register an unrelated standalone team on the bus at candidate + 4 seconds (140004) under temporary HOME
  jq -n --arg home "$AGENTS_HOME" --arg wt "$AGENTS_HOME/worktrees/repo/20261023-140004-other" \
    '{id: "20261023-140004-team", repo: "repo", wake_cmd: "agent", home: $home,
      members: {other: {agent: "other", task: "other-task", branch: "other-branch", worktree: $wt, state: "active"}}}' \
    | "$HOME/.local/bin/agentbus" team-create --dir "$AGENTS_HOME/bus"

  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  run timeout --kill-after=2s 15s agent run --resume "$team" "second task"
  [ "$status" -eq 0 ]

  new_lead=$(task_of lead)
  new_team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$new_lead/meta.env")

  # Must advance past 140000, 140001 (taskdir), 140002 (worktree), 140003 (branch), and 140004 (team)
  [ "$new_lead" = "20261023-140005-lead" ]
  [ "$new_team" = "20261023-140005-team" ]

  # All 4 pre-existing obstacles must be preserved untouched
  [ -f "$AGENTS_HOME/tasks/20261023-140001-lead/sentinel.txt" ]
  [ "$(<"$AGENTS_HOME/tasks/20261023-140001-lead/sentinel.txt")" = "taskdir data" ]

  [ -f "$AGENTS_HOME/worktrees/repo/20261023-140002-lead/sentinel.txt" ]
  [ "$(<"$AGENTS_HOME/worktrees/repo/20261023-140002-lead/sentinel.txt")" = "worktree data" ]

  git -C "$HOME/repo" show-ref --verify --quiet "refs/heads/agents/lead/20261023-140003"

  standalone_team=$("$HOME/.local/bin/agentbus" team-get --dir "$AGENTS_HOME/bus" --team "20261023-140004-team")
  [ "$(jq -r .id <<< "$standalone_team")" = "20261023-140004-team" ]
  [ "$(jq -r .home <<< "$standalone_team")" = "$AGENTS_HOME" ]

  wait_status "$new_lead" 'done'
}

@test "same-second solo task resume allocates fresh ID and preserves predecessor" {
  install_date_shim
  export MOCK_TIMESTAMP="20261023-160000"

  scenario '{"solo": {"new": "sleep:300"}}'
  agent run solo "solo task part 1"
  old_task=$(task_of solo)
  wait_status "$old_task" running
  agent stop --running
  wait_status "$old_task" stopped
  wait_task_runner_exit "$old_task"

  cp "$AGENTS_HOME/tasks/$old_task/meta.env" "$HOME/old_solo_meta.env"
  cp "$AGENTS_HOME/tasks/$old_task/task.md" "$HOME/old_solo_task.md"

  scenario '{"solo": {"new": "done"}}'
  run timeout --kill-after=2s 15s agent run --resume "$old_task" "solo task part 2"
  [ "$status" -eq 0 ]

  new_task=$(task_of solo)
  [ "$new_task" != "$old_task" ]
  [ -d "$AGENTS_HOME/tasks/$old_task" ]
  [ -d "$AGENTS_HOME/tasks/$new_task" ]

  cmp -s "$AGENTS_HOME/tasks/$old_task/meta.env" "$HOME/old_solo_meta.env"
  cmp -s "$AGENTS_HOME/tasks/$old_task/task.md" "$HOME/old_solo_task.md"
  [ "$(sed -n 's/^RESUMED_FROM=//p' "$AGENTS_HOME/tasks/$new_task/meta.env")" = "$old_task" ]
  [ -z "$(sed -n 's/^RESUMED_FROM=//p' "$AGENTS_HOME/tasks/$old_task/meta.env")" ]

  grep -q "solo task part 1" "$AGENTS_HOME/tasks/$new_task/task.md"
  grep -q "solo task part 2" "$AGENTS_HOME/tasks/$new_task/task.md"

  wait_status "$new_task" 'done'
}

@test "two resumed teams with disjoint agents avoid colliding on the same new team ID under concurrency" {
  install_date_shim

  # Start and stop team 1 under timestamp 180000
  export MOCK_TIMESTAMP="20261023-180000"
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team1 task"
  t1_lead=$(task_of lead)
  t1_member=$(task_of member)
  wait_status "$t1_lead" running
  wait_status "$t1_member" running
  agent stop --running
  wait_status "$t1_lead" stopped
  wait_status "$t1_member" stopped
  wait_task_runner_exit "$t1_lead"
  wait_task_runner_exit "$t1_member"
  team1=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$t1_lead/meta.env")

  # Start and stop team 2 (disjoint agents: solo, worker) under timestamp 180001
  export MOCK_TIMESTAMP="20261023-180001"
  scenario '{"solo": {"new": "sleep:300"}, "worker": {"new": "sleep:300"}}'
  agent run --team solo,worker "team2 task"
  t2_solo=$(task_of solo)
  t2_worker=$(task_of worker)
  wait_status "$t2_solo" running
  wait_status "$t2_worker" running
  agent stop --running
  wait_status "$t2_solo" stopped
  wait_status "$t2_worker" stopped
  wait_task_runner_exit "$t2_solo"
  wait_task_runner_exit "$t2_worker"
  team2=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$t2_solo/meta.env")

  # Launch both resumes concurrently targeting candidate timestamp 180010
  export MOCK_TIMESTAMP="20261023-180010"
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}, "solo": {"new": "done"}, "worker": {"new": "done"}}'
  timeout --kill-after=2s 15s agent run --resume "$team1" "team1 resumed" > "$HOME/resume1.log" 2>&1 &
  pid1=$!
  timeout --kill-after=2s 15s agent run --resume "$team2" "team2 resumed" > "$HOME/resume2.log" 2>&1 &
  pid2=$!

  status1=0
  status2=0
  wait "$pid1" || status1=$?
  wait "$pid2" || status2=$?

  [ "$status1" -eq 0 ]
  [ "$status2" -eq 0 ]

  new_t1_lead=$(task_of lead)
  new_t2_solo=$(task_of solo)
  new_team1=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$new_t1_lead/meta.env")
  new_team2=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$new_t2_solo/meta.env")

  # The two concurrently resumed teams must have distinct team IDs
  [ "$new_team1" != "$new_team2" ]

  # All eight task directories must exist
  [ -d "$AGENTS_HOME/tasks/$t1_lead" ]
  [ -d "$AGENTS_HOME/tasks/$t1_member" ]
  [ -d "$AGENTS_HOME/tasks/$t2_solo" ]
  [ -d "$AGENTS_HOME/tasks/$t2_worker" ]
  [ -d "$AGENTS_HOME/tasks/$new_t1_lead" ]
  [ -d "$AGENTS_HOME/tasks/$(task_of member)" ]
  [ -d "$AGENTS_HOME/tasks/$new_t2_solo" ]
  [ -d "$AGENTS_HOME/tasks/$(task_of worker)" ]

  wait_status "$new_t1_lead" 'done'
  wait_status "$(task_of member)" 'done'
  wait_status "$new_t2_solo" 'done'
  wait_status "$(task_of worker)" 'done'
}

@test "team creation advances past an orphaned task directory with different member name" {
  install_date_shim
  export MOCK_TIMESTAMP="20261023-150000"

  # An orphaned task directory from a crashed run exists at the candidate timestamp (candidate-other)
  # with no metadata and no team registration on the bus
  mkdir -p "$AGENTS_HOME/tasks/20261023-150000-other"
  echo "orphaned sentinel" > "$AGENTS_HOME/tasks/20261023-150000-other/sentinel.txt"

  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  run timeout --kill-after=2s 15s agent run --team lead,member "team task with orphan prefix"
  [ "$status" -eq 0 ]

  new_lead=$(task_of lead)
  new_member=$(task_of member)
  new_team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$new_lead/meta.env")

  # Conservative whole-cohort timestamp collision avoidance:
  # The entire timestamp 20261023-150000 must be skipped because of the orphaned 20261023-150000-other directory.
  # The new team and tasks must advance to 20261023-150001.
  [ "$new_lead" = "20261023-150001-lead" ]
  [ "$new_member" = "20261023-150001-member" ]
  [ "$new_team" = "20261023-150001-team" ]

  # The orphaned directory and its contents must remain untouched
  [ -f "$AGENTS_HOME/tasks/20261023-150000-other/sentinel.txt" ]
  [ "$(<"$AGENTS_HOME/tasks/20261023-150000-other/sentinel.txt")" = "orphaned sentinel" ]

  wait_status "$new_lead" 'done'
  wait_status "$new_member" 'done'
}
