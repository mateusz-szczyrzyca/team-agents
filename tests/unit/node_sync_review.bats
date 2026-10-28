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
    if [[ -f "$STUB_HOME/sync-fail-after-n" ]]; then
      seen=0
      if [[ -f "$STUB_HOME/sync-req-count" ]]; then
        seen=$(< "$STUB_HOME/sync-req-count")
      fi
      seen=$((seen + 1))
      printf '%d' "$seen" > "$STUB_HOME/sync-req-count"
      limit=$(< "$STUB_HOME/sync-fail-after-n")
      if (( seen > limit )); then
        exit 1
      fi
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
    echo "{\"ts\":\"2026-10-28T12:00:0$i\",\"level\":\"info\",\"event\":\"unselected_event\"}" >> "$AGENTS_HOME/logs/agentbus.jsonl"
  done

  agent node --once
  [ ! -f "$STUB_HOME/last-sync.json" ]
  cursor=$(jq -r '.["agentbus.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq 5 ]

  for i in {6..9}; do
    echo "{\"ts\":\"2026-10-28T12:00:0$i\",\"level\":\"info\",\"event\":\"unselected_event\"}" >> "$AGENTS_HOME/logs/agentbus.jsonl"
  done
  echo "{\"ts\":\"2026-10-28T12:00:10\",\"level\":\"info\",\"event\":\"unselected_event\"}" >> "$AGENTS_HOME/logs/agentbus.jsonl"

  agent node --once
  [ ! -f "$STUB_HOME/last-sync.json" ]
  cursor=$(jq -r '.["agentbus.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq 10 ]
}

@test "M1: finished unchanged task with job transitions and locks fast-skips without reading artifacts" {
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
  # Include job transitions so node_scan_tasks creates and updates meta.env.lock
  echo '{"state":"done","at":"2026-10-28T12:00:00"}' > "$AGENTS_HOME/tasks/T1/job_transitions.jsonl"
  touch "$AGENTS_HOME/tasks/T1/meta.env.lock"

  agent node --once
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
  status=$(jq -r '.metadata.STATUS' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ "$status" = "done" ]
  rm -f "$STUB_HOME/last-sync.json"

  # Manifest is newer than task artifacts
  touch -d '1 hour ago' "$AGENTS_HOME/tasks/T1/final.md"
  touch "$AGENTS_HOME/tasks/T1/sync_manifest.json"

  # Setup sha256sum spy to detect whether task artifacts are re-hashed
  cat > "$HOME/.local/bin/sha256sum" <<'SPY'
#!/usr/bin/env bash
echo "$@" >> "$STUB_HOME/sha-calls.log"
exec /usr/bin/sha256sum "$@"
SPY
  chmod +x "$HOME/.local/bin/sha256sum"

  echo '{"ts":"2026-10-28T12:00:00","level":"warn","event":"test_node_warn"}' >> "$AGENTS_HOME/logs/agent.jsonl"

  agent node --once

  [ -f "$STUB_HOME/last-sync.json" ]
  warn_event=$(jq -r '.Logs[0].event // empty' "$STUB_HOME/last-sync.json")
  [ "$warn_event" = "test_node_warn" ]

  # Unchanged terminal task must NOT have re-hashed its artifacts
  [ ! -f "$STUB_HOME/sha-calls.log" ]
}

@test "M1: old manifest without complete field is promoted on clean scan" {
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
JOB_ID=job-1
RESTARTS=0
EOF
  echo "final output" > "$AGENTS_HOME/tasks/T1/final.md"
  final_sha=$(sha256sum "$AGENTS_HOME/tasks/T1/final.md" | awk '{print $1}')

  # Write legacy manifest without complete field
  cat > "$AGENTS_HOME/tasks/T1/sync_manifest.json" <<EOF
{
  "metadata": {
    "STATUS": "done",
    "RUN": "1",
    "AGENT": "solo",
    "TYPE": "claude",
    "MODEL": "test",
    "JOB_ID": "job-1",
    "RESTARTS": "0"
  },
  "files": {
    "final.md": {"sha": "$final_sha", "cut": false}
  }
}
EOF

  # Run poll: should see no changes and promote manifest to complete: true
  agent node --once

  promoted=$(jq -r '.complete // false' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ "$promoted" = "true" ]
}

@test "M1: partial sync failure leaves complete=false so remaining older files are synced on next poll" {
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

  # Create 9 prompt files of 250 KiB each (forcing chunked requests > 1.5 MiB budget)
  for i in $(seq 1 9); do
    head -c 256000 < /dev/zero | tr '\0' 'A' > "$AGENTS_HOME/tasks/T1/prompt-$i.md"
  done

  # Allow only 1 sync request to succeed, then fail on the second
  echo 1 > "$STUB_HOME/sync-fail-after-n"

  agent node --once || true

  # Partial manifest must have been saved with complete: false
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
  complete_status=$(jq -r '.complete // false' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ "$complete_status" = "false" ]

  # Manifest is newer than files
  touch "$AGENTS_HOME/tasks/T1/sync_manifest.json"

  # Remove failure injection
  rm -f "$STUB_HOME/sync-fail-after-n" "$STUB_HOME/sync-req-count" "$STUB_HOME/last-sync.json"

  # Next poll must NOT fast-skip; it must send the remaining files
  agent node --once

  [ -f "$STUB_HOME/last-sync.json" ]
  sent_name=$(jq -r '.Files[0].Name // empty' "$STUB_HOME/last-sync.json")
  [ -n "$sent_name" ]

  # Manifest now complete
  complete_status=$(jq -r '.complete // false' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ "$complete_status" = "true" ]
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

@test "M2: valid job before stale job recovers in second pass after stale job is marked obsolete" {
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
    if [[ $jid == "job-z-stale" ]]; then
      # Unknown to lead: exit 3 BUS_NOT_FOUND (or 5 BUS_PERMISSION)
      exit 3
    fi
    # If the request contains unowned info logs for Tstale, lead returns exit 2 (LeadSyncInvalidLogs / InvalidArgument)
    has_stale_log=$(jq '[.Logs[] | select((.level != "warn" and .level != "error") and .task == "Tstale")] | length' "$req_file")
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

  # job-a-valid is lexicographically before job-z-stale
  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/Tstale" "$AGENTS_HOME/tasks/Tvalid" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{
  "jobs": {
    "job-a-valid": {"assignment": 2, "task_id": "Tvalid", "launched": true, "state": "running"},
    "job-z-stale": {"assignment": 1, "task_id": "Tstale", "launched": true, "state": "done"}
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

  is_obsolete=$(jq -r '.jobs["job-z-stale"].obsolete // false' "$AGENTS_HOME/bus/node-jobs.json")
  [ "$is_obsolete" = "true" ]

  [ -f "$STUB_HOME/last-valid-sync.json" ]
  valid_jid=$(jq -r '.Job' "$STUB_HOME/last-valid-sync.json")
  [ "$valid_jid" = "job-a-valid" ]
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

  echo '{"ts":"2026-10-28T12:00:00","level":"warn","event":"warn_1"}' >> "$AGENTS_HOME/logs/agentbus.jsonl"
  echo '' >> "$AGENTS_HOME/logs/agentbus.jsonl"
  echo '{"ts":"2026-10-28T12:00:02","level":"warn","event":"warn_2"}' >> "$AGENTS_HOME/logs/agentbus.jsonl"

  agent node --once
  [ -f "$STUB_HOME/last-sync.json" ]
  num_logs=$(jq '.Logs | length' "$STUB_HOME/last-sync.json")
  [ "$num_logs" -eq 2 ]

  total_l=$(wc -l < "$AGENTS_HOME/logs/agentbus.jsonl")
  cursor=$(jq -r '.["agentbus.jsonl"] // 0' "$AGENTS_HOME/bus/sync_log_cursors.json")
  [ "$cursor" -eq "$total_l" ]

  rm -f "$STUB_HOME/last-sync.json"
  agent node --once
  [ ! -f "$STUB_HOME/last-sync.json" ]
}

@test "a terminal artifact read failure keeps the snapshot incomplete until retry" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1"
  echo '{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"done"}},"outbox":[]}' > "$AGENTS_HOME/bus/node-jobs.json"
  printf 'STATUS=done\nRUN=1\nAGENT=solo\nTYPE=claude\nMODEL=test\n' > "$AGENTS_HOME/tasks/T1/meta.env"
  printf 'retry this final\n' > "$AGENTS_HOME/tasks/T1/final.md"
  touch -d '2020-01-01 UTC' "$AGENTS_HOME/tasks/T1/final.md"
  chmod 000 "$AGENTS_HOME/tasks/T1/final.md"
  run agent node --once
  chmod 644 "$AGENTS_HOME/tasks/T1/final.md"
  [ "$status" -eq 0 ]
  [ "$(jq -r '.complete' "$AGENTS_HOME/tasks/T1/sync_manifest.json")" = false ]
  agent node --once
  [ "$(jq -r '.complete' "$AGENTS_HOME/tasks/T1/sync_manifest.json")" = true ]
  [ "$(jq -r '.Files[] | select(.Name == "final.md") | .Content' "$STUB_HOME/last-sync.json")" = 'retry this final' ]
}
