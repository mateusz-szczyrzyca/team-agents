#!/usr/bin/env bats
# Tests for node sync request batching limits, shared log cursors, and incremental persistence.

load helpers

# setup_limits_sync_bus_stub intercepts node-request calls to /v1/heartbeat and /v1/sync,
# recording every sync request payload as a line in $STUB_HOME/sync-requests.jsonl.
setup_limits_sync_bus_stub() {
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
    req_file="$STUB_HOME/req.tmp"
    cat > "$req_file"
    req_len=$(wc -c < "$req_file")
    if [[ -f "$STUB_HOME/sync-fail-after-n" ]]; then
      seen=0
      if [[ -f "$STUB_HOME/sync-req-count" ]]; then
        seen=$(< "$STUB_HOME/sync-req-count")
      fi
      seen=$((seen + 1))
      printf '%d' "$seen" > "$STUB_HOME/sync-req-count"
      limit=$(< "$STUB_HOME/sync-fail-after-n")
      if (( seen > limit )); then
        rm -f "$req_file"
        exit 1
      fi
    fi
    jq -c --arg len "$req_len" '. + {_wire_len: ($len|tonumber)}' < "$req_file" >> "$STUB_HOME/sync-requests.jsonl"
    rm -f "$req_file"
    printf '{"OK":true}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"
}

@test "node sync handles >2MiB aggregate files and logs via bounded requests <= 2MiB" {
  setup_limits_sync_bus_stub
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
MODEL=claude-3-5
NOTE='batch sync test'
PAUSE_REASON=
WAKE_AT=
CREATED=2026-10-28T10:00:00Z
FINISHED=
JOB_ID=job-1
RESTARTS=0
EOF

  # Create 6 prompt files of 250 KiB each (total ~1.5 MiB files)
  for i in $(seq 1 6); do
    head -c 256000 < /dev/zero | tr '\0' 'A' > "$AGENTS_HOME/tasks/T1/prompt-$i.md"
  done

  # Create ~1.2 MiB of warning logs (aggregate files + logs is ~2.7 MiB)
  for j in $(seq 1 20); do
    head -c 60000 < /dev/zero | tr '\0' 'x' | jq -c -Rs --arg idx "$j" '{ts: "2026-10-28 10:00:00", level: "warn", msg: ., id: $idx}' >> "$AGENTS_HOME/logs/agent.jsonl"
  done

  run agent node --once
  [ "$status" -eq 0 ]

  # Must have captured sync requests
  [ -f "$STUB_HOME/sync-requests.jsonl" ]
  req_count=$(wc -l < "$STUB_HOME/sync-requests.jsonl")
  # Aggregate > 2 MiB cannot be sent in a single request
  [ "$req_count" -ge 2 ]

  # Every captured request must be bounded at <= 2 MiB (2097152 bytes)
  while IFS= read -r line; do
    wire_len=$(jq -r '._wire_len' <<< "$line")
    [ "$wire_len" -le 2097152 ]
  done < "$STUB_HOME/sync-requests.jsonl"

  # All 6 files must be received across the requests
  received_files=$(jq -s '[.[].Files[].Name] | unique | length' "$STUB_HOME/sync-requests.jsonl")
  [ "$received_files" -eq 6 ]

  # Sync manifest must record all 6 files
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
  manifest_files=$(jq '.files | length' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ "$manifest_files" -eq 6 ]
}

@test "node sync survives shared log cursor across multiple job task IDs and log files" {
  setup_limits_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1" "$AGENTS_HOME/tasks/T2" "$AGENTS_HOME/logs"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"},"job-2":{"assignment":2,"task_id":"T2","launched":true,"state":"running"}},"outbox":[]}
EOF

  for tid in T1 T2; do
    cat > "$AGENTS_HOME/tasks/$tid/meta.env" <<EOF
STATUS=running
RUN=1
AGENT=solo
TYPE=claude
MODEL=claude-3-5
NOTE='log cursor test'
PAUSE_REASON=
WAKE_AT=
CREATED=2026-10-28T10:00:00Z
FINISHED=
JOB_ID=job-$tid
RESTARTS=0
EOF
  done

  # Add logs across all 3 log files: agent.jsonl, agentbus.jsonl, agentlive.jsonl
  for f in agent.jsonl agentbus.jsonl agentlive.jsonl; do
    cat > "$AGENTS_HOME/logs/$f" <<EOF
{"ts":"2026-10-28 10:00:01","level":"warn","event":"shared_warn_$f"}
{"ts":"2026-10-28 10:00:02","level":"info","task":"T1","event":"t1_info_$f"}
{"ts":"2026-10-28 10:00:03","level":"info","task":"T2","event":"t2_info_$f"}
EOF
  done

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/sync-requests.jsonl" ]

  # Check that T1 info, T2 info, and warn events from all 3 log files were captured
  all_logs=$(jq -s '[.[].Logs[]]' "$STUB_HOME/sync-requests.jsonl")
  for f in agent.jsonl agentbus.jsonl agentlive.jsonl; do
    [ "$(jq -r --arg ev "shared_warn_$f" '[.[] | select(.event == $ev)] | length' <<< "$all_logs")" -ge 1 ]
    [ "$(jq -r --arg ev "t1_info_$f" '[.[] | select(.event == $ev)] | length' <<< "$all_logs")" -ge 1 ]
    [ "$(jq -r --arg ev "t2_info_$f" '[.[] | select(.event == $ev)] | length' <<< "$all_logs")" -ge 1 ]
  done

  # A second sync poll should NOT resend the already-synced warnings or info logs
  rm -f "$STUB_HOME/sync-requests.jsonl"
  run agent node --once
  [ "$status" -eq 0 ]
  if [[ -f "$STUB_HOME/sync-requests.jsonl" ]]; then
    second_logs=$(jq -s '[.[].Logs[]]' "$STUB_HOME/sync-requests.jsonl")
    for f in agent.jsonl agentbus.jsonl agentlive.jsonl; do
      [ "$(jq -r --arg ev "shared_warn_$f" '[.[] | select(.event == $ev)] | length' <<< "$second_logs")" -eq 0 ]
      [ "$(jq -r --arg ev "t1_info_$f" '[.[] | select(.event == $ev)] | length' <<< "$second_logs")" -eq 0 ]
      [ "$(jq -r --arg ev "t2_info_$f" '[.[] | select(.event == $ev)] | length' <<< "$second_logs")" -eq 0 ]
    done
  fi
}

@test "node sync persists partial batch ACK hashes when subsequent request fails" {
  setup_limits_sync_bus_stub
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
MODEL=claude-3-5
NOTE='partial ack test'
PAUSE_REASON=
WAKE_AT=
CREATED=2026-10-28T10:00:00Z
FINISHED=
JOB_ID=job-1
RESTARTS=0
EOF

  # Create 8 prompt files of 250 KiB each (forcing chunked requests)
  for i in $(seq 1 8); do
    head -c 256000 < /dev/zero | tr '\0' 'A' > "$AGENTS_HOME/tasks/T1/prompt-$i.md"
  done

  # Allow exactly 1 sync request to succeed, then fail on the second
  echo 1 > "$STUB_HOME/sync-fail-after-n"

  run agent node --once
  [ "$status" -eq 0 ]

  # Exactly 1 request was accepted
  [ -f "$STUB_HOME/sync-requests.jsonl" ]
  [ "$(wc -l < "$STUB_HOME/sync-requests.jsonl")" -eq 1 ]

  first_file=$(head -n 1 "$STUB_HOME/sync-requests.jsonl" | jq -r '.Files[0].Name // empty')
  [ -n "$first_file" ]

  # The accepted file in the first request MUST be recorded in sync_manifest.json
  # even though subsequent requests failed.
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
  recorded_sha=$(jq -r --arg f "$first_file" '.files[$f].sha // empty' "$AGENTS_HOME/tasks/T1/sync_manifest.json")
  [ -n "$recorded_sha" ]
}
