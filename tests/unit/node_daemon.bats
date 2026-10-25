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

# setup_node_bus_stub intercepts node-request to provide deterministic lead responses
setup_node_bus_stub() {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
if [[ ${1:-} == node-request ]]; then
  shift
  path=""
  validate=0
  while (($#)); do
    case $1 in
      --path) path=$2; shift 2 ;;
      --validate) validate=1; shift ;;
      *) shift ;;
    esac
  done
  if ((validate)); then
    cat >/dev/null
    printf '{}\n'
    exit 0
  fi
  if [[ $path == "/v1/heartbeat" ]]; then
    cat > "$STUB_HOME/last-heartbeat.json"
    if [[ ! -f "$STUB_HOME/job-delivered" ]]; then
      touch "$STUB_HOME/job-delivered"
      cat <<'EOF'
{
  "Job": {
    "Request": {
      "ID": "job-test-1",
      "Repository": "repo",
      "BaseRef": "main",
      "BaseSHA": "",
      "Requires": [],
      "Selector": "solo",
      "Task": "say hello"
    },
    "Assignment": 1,
    "Node": "node-test"
  },
  "Stops": []
}
EOF
    else
      printf '{"Job":null,"Stops":[]}\n'
    fi
    exit 0
  fi
  if [[ $path == /v1/jobs/*/state ]]; then
    cat > "$STUB_HOME/last-state.json"
    printf '{"State":"running"}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"
}

@test "agent node polls heartbeat, prepares assigned job, acks running state before launching" {
  setup_node_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$AGENTS_HOME/bus/node-jobs.json" ]
  [ -f "$HOME/last-state.json" ]
  grep -q '"State":"running"' "$HOME/last-state.json"
  grep -q '"Assignment":1' "$HOME/last-state.json"
  local task_id
  task_id=$(jq -r '.jobs["job-test-1"].task_id' "$AGENTS_HOME/bus/node-jobs.json")
  [ -n "$task_id" ]
  [ -f "$AGENTS_HOME/tasks/$task_id/meta.env" ]
  grep -q 'JOB_ID=job-test-1' "$AGENTS_HOME/tasks/$task_id/meta.env"
  grep -q 'JOB_ASSIGNMENT=1' "$AGENTS_HOME/tasks/$task_id/meta.env"
}

@test "second agent node daemon fails immediately when another daemon is running" {
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  agent node > "$HOME/node.log" 2>&1 &
  local pid=$!
  echo "$pid" > "$HOME/node.pid"
  wait_grep '"event":"node_started"' "$AGENTS_HOME/logs/agent.jsonl" 15
  run agent node
  [ "$status" -ne 0 ]
  [[ $output == *"another node daemon is already running"* ]]
  kill -TERM "$pid"
  local rc=0
  wait "$pid" || rc=$?
  [ "$rc" -eq 0 ]
  rm "$HOME/node.pid"
}

@test "agent node fails closed on corrupt node-jobs.json without overwriting it" {
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  printf '{"jobs": corrupt' > "$AGENTS_HOME/bus/node-jobs.json"
  run agent node --once
  [ "$status" -ne 0 ]
  [[ $output == *"corrupt"* ]]
  [ "$(cat "$AGENTS_HOME/bus/node-jobs.json")" = '{"jobs": corrupt' ]
}

@test "agent node handles checkout failure by reporting failed and marking nothing started" {
  setup_node_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  # NODE_REPOS empty: checkout fails
  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$AGENTS_HOME/bus/node-jobs.json" ]
  grep -q '"State":"failed"' "$AGENTS_HOME/bus/node-jobs.json"
}

@test "agent node restart recovery does not duplicate launch" {
  setup_node_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  run agent node --once
  [ "$status" -eq 0 ]
  local task_id
  task_id=$(jq -r '.jobs["job-test-1"].task_id' "$AGENTS_HOME/bus/node-jobs.json")
  grep -q '^RUN=1' "$AGENTS_HOME/tasks/$task_id/meta.env"

  # Second poll tick / restart
  run agent node --once
  [ "$status" -eq 0 ]
  # RUN remains 1, no duplicate runner was started
  grep -q '^RUN=1' "$AGENTS_HOME/tasks/$task_id/meta.env"
}

@test "agent node stops active task on stop request in heartbeat" {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
if [[ ${1:-} == node-request ]]; then
  shift
  path=""
  validate=0
  while (($#)); do
    case $1 in
      --path) path=$2; shift 2 ;;
      --validate) validate=1; shift ;;
      *) shift ;;
    esac
  done
  if ((validate)); then printf '{}\n'; exit 0; fi
  if [[ $path == "/v1/heartbeat" ]]; then
    if [[ ! -f "$STUB_HOME/job-delivered" ]]; then
      touch "$STUB_HOME/job-delivered"
      cat <<'EOF'
{"Job":{"Request":{"ID":"job-stop-1","Repository":"repo","BaseRef":"main","BaseSHA":"","Requires":[],"Selector":"solo","Task":"text"},"Assignment":1,"Node":"node-test"},"Stops":[]}
EOF
    else
      cat <<'EOF'
{"Job":null,"Stops":[{"Request":{"ID":"job-stop-1"},"Assignment":1}]}
EOF
    fi
    exit 0
  fi
  if [[ $path == /v1/jobs/*/state ]]; then
    printf '{"State":"running"}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"

  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  # Tick 1: prepares and starts job-stop-1
  run agent node --once
  [ "$status" -eq 0 ]
  local task_id
  task_id=$(jq -r '.jobs["job-stop-1"].task_id' "$AGENTS_HOME/bus/node-jobs.json")
  [ -n "$task_id" ]

  # Tick 2: receives Stops for job-stop-1
  run agent node --once
  [ "$status" -eq 0 ]
  grep -q '^STATUS=stopped' "$AGENTS_HOME/tasks/$task_id/meta.env"
  grep -q '"State":"stopped"' "$AGENTS_HOME/bus/node-jobs.json"
}

@test "agent node preserves done outcome when completion wins race against stop request" {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
if [[ ${1:-} == node-request ]]; then
  shift
  path=""
  validate=0
  while (($#)); do
    case $1 in
      --path) path=$2; shift 2 ;;
      --validate) validate=1; shift ;;
      *) shift ;;
    esac
  done
  if ((validate)); then printf '{}\n'; exit 0; fi
  if [[ $path == "/v1/heartbeat" ]]; then
    if [[ ! -f "$STUB_HOME/job-delivered" ]]; then
      touch "$STUB_HOME/job-delivered"
      cat <<'EOF'
{"Job":{"Request":{"ID":"job-done-race","Repository":"repo","BaseRef":"main","BaseSHA":"","Requires":[],"Selector":"solo","Task":"text"},"Assignment":1,"Node":"node-test"},"Stops":[]}
EOF
    else
      cat <<'EOF'
{"Job":null,"Stops":[{"Request":{"ID":"job-done-race"},"Assignment":1}]}
EOF
    fi
    exit 0
  fi
  if [[ $path == /v1/jobs/*/state ]]; then
    printf '{"State":"running"}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"

  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  # Tick 1: prepares and starts job-done-race
  run agent node --once
  [ "$status" -eq 0 ]
  local task_id
  task_id=$(jq -r '.jobs["job-done-race"].task_id' "$AGENTS_HOME/bus/node-jobs.json")
  [ -n "$task_id" ]

  # Simulate completion before next heartbeat
  echo "STATUS=done" >> "$AGENTS_HOME/tasks/$task_id/meta.env"
  printf 'Success result text\n' > "$AGENTS_HOME/tasks/$task_id/final.md"

  # Tick 2: receives Stops for job-done-race
  run agent node --once
  [ "$status" -eq 0 ]
  # Done status must be preserved: terminal result wins race
  grep -q '^STATUS=done' "$AGENTS_HOME/tasks/$task_id/meta.env"
}

@test "agent node preserves outbox when lead returns malformed reply" {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
if [[ ${1:-} == node-request ]]; then
  shift
  path=""
  validate=0
  while (($#)); do
    case $1 in
      --path) path=$2; shift 2 ;;
      --validate) validate=1; shift ;;
      *) shift ;;
    esac
  done
  if ((validate)); then printf '{}\n'; exit 0; fi
  if [[ $path == "/v1/heartbeat" ]]; then
    echo "malformed-not-json"
    exit 1
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"

  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  # Put a pending report in outbox
  mkdir -p "$AGENTS_HOME/bus"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{},"outbox":[{"ID":"job-pending","State":"running"}]}
EOF

  run agent node --once
  # Outbox must not be cleared on failure
  [ "$(jq -r '.outbox | length' "$AGENTS_HOME/bus/node-jobs.json")" = "1" ]
}
