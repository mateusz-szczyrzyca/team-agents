#!/usr/bin/env bats
# Tests for review findings C1, M1, M2, and m3 on node incremental sync and log cursors.

load helpers

# setup_node_sync_bus_stub intercepts node-request calls to /v1/heartbeat and /v1/sync.
setup_node_sync_bus_stub() {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
if [[ ${1:-} == "node-request" ]]; then
  shift
  path=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --path) path="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [[ $path == "/v1/heartbeat" ]]; then
    cat > "$STUB_HOME/last-heartbeat.json"
    printf '{"Job":null,"Stops":[]}\n'
    exit 0
  fi
  if [[ $path == "/v1/sync" ]]; then
    if [[ -f "$STUB_HOME/sync-fail" ]]; then
      exit 1
    fi
    cat > "$STUB_HOME/last-sync.json"
    printf '{"OK":true}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"
}

@test "C1: idle node advances log cursor across unselected info entries without sending sync" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}
EOF
  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=running
RUN=1
AGENT=solo
TYPE=claude
MODEL=test
EOF
  agent node --once
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
  rm -f "$STUB_HOME/last-sync.json"

  for i in {1..5}; do
    echo "{\"ts\":\"2026-10-28T12:00:0$i\",\"level\":\"info\",\"event\":\"node_heartbeat_accepted\"}" >> "$AGENTS_HOME/logs/agent.jsonl"
  done

  agent node --once
  [ ! -f "$STUB_HOME/last-sync.json" ]
  cursor=$(jq -r '.["agent.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq 5 ]

  for i in {6..9}; do
    echo "{\"ts\":\"2026-10-28T12:00:0$i\",\"level\":\"info\",\"event\":\"node_heartbeat_accepted\"}" >> "$AGENTS_HOME/logs/agent.jsonl"
  done
  echo "{\"ts\":\"2026-10-28T12:00:10\",\"level\":\"info\",\"event\":\"node_heartbeat_accepted\"}" >> "$AGENTS_HOME/logs/agent.jsonl"

  agent node --once
  [ ! -f "$STUB_HOME/last-sync.json" ]
  cursor=$(jq -r '.["agent.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq 10 ]
}

@test "M1: finished unchanged task is fast-skipped without reading files while node logs still sync" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"done"}},"outbox":[]}
EOF
  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=done
RUN=1
AGENT=solo
TYPE=claude
MODEL=test
EOF
  echo "done" > "$AGENTS_HOME/tasks/T1/final.md"

  agent node --once
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
  status=$(jq -r '.metadata.STATUS' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ "$status" = "done" ]
  rm -f "$STUB_HOME/last-sync.json"

  sleep 1
  touch "$AGENTS_HOME/tasks/T1/sync_manifest.json"

  # Fast-skip must not read or diff meta.env for finished unchanged task
  chmod 000 "$AGENTS_HOME/tasks/T1/meta.env"

  echo '{"ts":"2026-10-28T12:00:00","level":"warn","event":"test_node_warn"}' >> "$AGENTS_HOME/logs/agent.jsonl"

  agent node --once

  [ -f "$STUB_HOME/last-sync.json" ]
  warn_event=$(jq -r '.Logs[0].event // empty' "$STUB_HOME/last-sync.json")
  [ "$warn_event" = "test_node_warn" ]

  cursor=$(jq -r '.["agent.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq 1 ]

  chmod 644 "$AGENTS_HOME/tasks/T1/meta.env"
}

@test "M2: task ID exact match excludes local non-job task whose ID is a substring" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T10" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-10":{"assignment":1,"task_id":"T10","launched":true,"state":"running"}},"outbox":[]}
EOF
  cat > "$AGENTS_HOME/tasks/T10/meta.env" <<'EOF'
STATUS=running
RUN=1
AGENT=solo
TYPE=claude
MODEL=test
EOF
  agent node --once
  rm -f "$STUB_HOME/last-sync.json"

  echo '{"ts":"2026-10-28T12:00:00","level":"info","task":"T1","event":"local_non_job_task"}' >> "$AGENTS_HOME/logs/agent.jsonl"
  echo '{"ts":"2026-10-28T12:00:01","level":"info","task":"T10","event":"job_task_log"}' >> "$AGENTS_HOME/logs/agent.jsonl"

  agent node --once
  [ -f "$STUB_HOME/last-sync.json" ]
  log_count=$(jq '.Logs | length' "$STUB_HOME/last-sync.json")
  [ "$log_count" -eq 1 ]
  log_task=$(jq -r '.Logs[0].task' "$STUB_HOME/last-sync.json")
  [ "$log_task" = "T10" ]
}

@test "M2: stale unknown job exits BUS_NOT_FOUND, is marked obsolete, and does not block valid job sync" {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
if [[ ${1:-} == "node-request" ]]; then
  shift
  path=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --path) path="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [[ $path == "/v1/heartbeat" ]]; then
    printf '{"Job":null,"Stops":[]}\n'
    exit 0
  fi
  if [[ $path == "/v1/sync" ]]; then
    req_file="$STUB_HOME/req-$RANDOM.json"
    cat > "$req_file"
    jid=$(jq -r '.Job' "$req_file")
    if [[ $jid == "job-stale" ]]; then
      exit 3
    fi
    has_stale_log=$(jq '[.Logs[] | select(.task == "Tstale")] | length' "$req_file")
    if (( has_stale_log > 0 )); then
      exit 2
    fi
    cat "$req_file" > "$STUB_HOME/last-valid-sync.json"
    printf '{"OK":true}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"

  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/Tstale" "$AGENTS_HOME/tasks/Tvalid" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{
  "jobs": {
    "job-stale": {"assignment": 1, "task_id": "Tstale", "launched": true, "state": "done"},
    "job-valid": {"assignment": 2, "task_id": "Tvalid", "launched": true, "state": "running"}
  },
  "outbox": []
}
EOF
  cat > "$AGENTS_HOME/tasks/Tstale/meta.env" <<'EOF'
STATUS=done
RUN=1
AGENT=solo
TYPE=claude
MODEL=test
EOF
  cat > "$AGENTS_HOME/tasks/Tvalid/meta.env" <<'EOF'
STATUS=running
RUN=1
AGENT=solo
TYPE=claude
MODEL=test
EOF

  echo '{"ts":"2026-10-28T12:00:00","level":"info","task":"Tstale","event":"stale_event"}' >> "$AGENTS_HOME/logs/agent.jsonl"
  echo '{"ts":"2026-10-28T12:00:01","level":"info","task":"Tvalid","event":"valid_event"}' >> "$AGENTS_HOME/logs/agent.jsonl"

  agent node --once

  is_obsolete=$(jq -r '.jobs["job-stale"].obsolete // false' "$AGENTS_HOME/bus/node-jobs.json")
  [ "$is_obsolete" = "true" ]

  [ -f "$STUB_HOME/last-valid-sync.json" ]
  valid_jid=$(jq -r '.Job' "$STUB_HOME/last-valid-sync.json")
  [ "$valid_jid" = "job-valid" ]
  log_task=$(jq -r '.Logs[0].task' "$STUB_HOME/last-valid-sync.json")
  [ "$log_task" = "Tvalid" ]
}

@test "m3: blank lines in log file advance cursor correctly without re-sending" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}
EOF
  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=running
RUN=1
AGENT=solo
TYPE=claude
MODEL=test
EOF
  agent node --once
  rm -f "$STUB_HOME/last-sync.json"

  echo '{"ts":"2026-10-28T12:00:00","level":"warn","event":"warn_1"}' >> "$AGENTS_HOME/logs/agent.jsonl"
  echo '' >> "$AGENTS_HOME/logs/agent.jsonl"
  echo '{"ts":"2026-10-28T12:00:02","level":"warn","event":"warn_2"}' >> "$AGENTS_HOME/logs/agent.jsonl"

  agent node --once
  [ -f "$STUB_HOME/last-sync.json" ]
  num_logs=$(jq '.Logs | length' "$STUB_HOME/last-sync.json")
  [ "$num_logs" -eq 2 ]

  total_l=$(wc -l < "$AGENTS_HOME/logs/agent.jsonl")
  cursor=$(jq -r '.["agent.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq "$total_l" ]

  rm -f "$STUB_HOME/last-sync.json"
  agent node --once
  [ ! -f "$STUB_HOME/last-sync.json" ]
}
