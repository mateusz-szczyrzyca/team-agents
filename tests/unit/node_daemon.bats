#!/usr/bin/env bats
# The node daemon (`agent node`). It connects to the lead over HTTP, sends periodic heartbeats,
# takes jobs matching its declared capabilities and free agents, starts tasks through the shared
# runner path, and recovers durably across crashes and restarts.

load helpers

teardown() {
  if [[ -f "$HOME/node.pid" ]]; then
    local node_pid
    node_pid=$(cat "$HOME/node.pid")
    if kill -0 "$node_pid" 2>/dev/null; then
      kill -TERM "$node_pid" 2>/dev/null || true
      wait "$node_pid" 2>/dev/null || true
    fi
    rm -f "$HOME/node.pid"
  fi
}

@test "agent node requires LEAD_URL and says how to set it" {
  run agent node
  [ "$status" -ne 0 ]
  [[ $output == *"LEAD_URL is not set: set it in config/agents.conf (e.g. LEAD_URL=http://lead-instance:7780)"* ]]
}

@test "agent node validates capabilities on startup" {
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_CAPABILITIES=(unknown:foo)' >> "$AGENTS_HOME/agents.conf"
  run agent node
  [ "$status" -ne 0 ]
  [[ $output == *"invalid capabilities:"* ]]
}

@test "agent node logs node_started and exits cleanly on SIGTERM" {
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  agent node > "$HOME/node.log" 2>&1 &
  local pid=$!
  echo "$pid" > "$HOME/node.pid"
  wait_grep '"event":"node_started"' "$AGENTS_HOME/logs/agent.jsonl" 15
  kill -TERM "$pid"
  local rc=0
  wait "$pid" || rc=$?
  [ "$rc" -eq 0 ]
  rm "$HOME/node.pid"
}

@test "prepare_job_task tags created task with JOB_ID and JOB_ASSIGNMENT" {
  source "$REPO_ROOT/bin/agent"
  # shellcheck disable=SC2034 # NODE_REPOS is read by prepare_job_task
  declare -gA NODE_REPOS=([repo]="$HOME/repo")
  cd "$HOME/repo" || return 1
  local first
  first=$(git rev-parse HEAD)
  local id
  id=$( ( exec 10>&-; prepare_job_task job-123 solo repo main "$first" "task text" 42 && printf '%s\n' "$NEW_ID" ) )
  [ -n "$id" ]
  [ "$(task_field "$id" JOB_ID)" = "job-123" ]
  [ "$(task_field "$id" JOB_ASSIGNMENT)" = "42" ]
}
