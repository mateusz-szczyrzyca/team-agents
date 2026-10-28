#!/usr/bin/env bats
# The node's incremental sync (POST /v1/sync): metadata, closed files, tails, limits, and logs.

load helpers

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

@test "node sync sends closed metadata and closed files on first poll, recording sync manifest" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}
EOF

  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=running
RUN=1
AGENT=solo
TYPE=claude
MODEL=claude-3-5
NOTE=doing sync
PAUSE_REASON=quota
WAKE_AT=1792735200
CREATED=2026-10-28T10:00:00Z
FINISHED=
JOB_ID=job-1
RESTARTS=0
SESSION_ID=secret-session
REPO_NAME=myrepo
BRANCH=topic
EOF
  printf '{"ts":"2026-10-28 10:00:01","k":"cmd","t":"make"}\n' > "$AGENTS_HOME/tasks/T1/activity.jsonl"
  printf 'hello prompt\n' > "$AGENTS_HOME/tasks/T1/prompt-1.md"
  printf 'unrelated\n' > "$AGENTS_HOME/tasks/T1/unrelated.txt"

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-sync.json" ]

  # Validated root fields
  [ "$(jq -r .Job "$STUB_HOME/last-sync.json")" = "job-1" ]
  [ "$(jq -r .Assignment "$STUB_HOME/last-sync.json")" = "1" ]
  [ "$(jq -r .TaskID "$STUB_HOME/last-sync.json")" = "T1" ]

  # Closed metadata list
  [ "$(jq -r .Metadata.STATUS "$STUB_HOME/last-sync.json")" = "running" ]
  [ "$(jq -r .Metadata.RUN "$STUB_HOME/last-sync.json")" = "1" ]
  [ "$(jq -r .Metadata.AGENT "$STUB_HOME/last-sync.json")" = "solo" ]
  [ "$(jq -r .Metadata.TYPE "$STUB_HOME/last-sync.json")" = "claude" ]
  [ "$(jq -r .Metadata.MODEL "$STUB_HOME/last-sync.json")" = "claude-3-5" ]
  [ "$(jq -r .Metadata.NOTE "$STUB_HOME/last-sync.json")" = "doing sync" ]
  [ "$(jq -r .Metadata.PAUSE_REASON "$STUB_HOME/last-sync.json")" = "quota" ]
  [ "$(jq -r .Metadata.WAKE_AT "$STUB_HOME/last-sync.json")" = "1792735200" ]
  [ "$(jq -r .Metadata.CREATED "$STUB_HOME/last-sync.json")" = "2026-10-28T10:00:00Z" ]
  [ "$(jq -r .Metadata.JOB_ID "$STUB_HOME/last-sync.json")" = "job-1" ]
  [ "$(jq -r .Metadata.RESTARTS "$STUB_HOME/last-sync.json")" = "0" ]

  # Keys outside closed list must not be sent
  run jq -e '.Metadata | has("SESSION_ID")' "$STUB_HOME/last-sync.json"
  [ "$status" -ne 0 ]
  run jq -e '.Metadata | has("REPO_NAME")' "$STUB_HOME/last-sync.json"
  [ "$status" -ne 0 ]
  run jq -e '.Metadata | has("BRANCH")' "$STUB_HOME/last-sync.json"
  [ "$status" -ne 0 ]

  # Closed file list
  [ "$(jq -r '.Files | length' "$STUB_HOME/last-sync.json")" -eq 2 ]
  [ "$(jq -r '.Files[] | select(.Name == "prompt-1.md") | .Content' "$STUB_HOME/last-sync.json")" = $'hello prompt\n' ]
  [ "$(jq -r '.Files[] | select(.Name == "prompt-1.md") | .Cut' "$STUB_HOME/last-sync.json")" = "false" ]
  run jq -e '.Files[] | select(.Name == "unrelated.txt")' "$STUB_HOME/last-sync.json"
  [ "$status" -ne 0 ]
  run jq -e '.Files[] | select(.Name == "meta.env")' "$STUB_HOME/last-sync.json"
  [ "$status" -ne 0 ]

  # Manifest written on disk
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
}

@test "node sync only sends changed files and metadata on subsequent sync" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}
EOF

  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=running
RUN=1
AGENT=solo
CREATED=2026-10-28T10:00:00Z
JOB_ID=job-1
EOF
  printf 'hello prompt\n' > "$AGENTS_HOME/tasks/T1/prompt-1.md"

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-sync.json" ]
  rm -f "$STUB_HOME/last-sync.json"

  # Second poll without changes sends no sync
  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/last-sync.json" ]

  # Now change only final.md
  printf 'done result\n' > "$AGENTS_HOME/tasks/T1/final.md"
  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-sync.json" ]

  # Files only contains final.md, not prompt-1.md
  [ "$(jq -r '.Files | length' "$STUB_HOME/last-sync.json")" -eq 1 ]
  [ "$(jq -r '.Files[0].Name' "$STUB_HOME/last-sync.json")" = "final.md" ]
  # Metadata has no changes
  [ "$(jq -r '.Metadata | length' "$STUB_HOME/last-sync.json")" -eq 0 ]
}

@test "node sync cuts files exceeding 256KiB and sets Cut: true" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}
EOF

  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=running
RUN=1
JOB_ID=job-1
EOF
  python3 -c 'print("x" * 300000)' > "$AGENTS_HOME/tasks/T1/activity.jsonl"

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-sync.json" ]
  [ "$(jq -r '.Files[0].Cut' "$STUB_HOME/last-sync.json")" = "true" ]
  local bytes
  bytes=$(jq -r '.Files[0].Content' "$STUB_HOME/last-sync.json" | wc -c)
  [ "$bytes" -le 262145 ]
}

@test "node sync sends warn/error logs and job task logs advancing cursor" {
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
JOB_ID=job-1
EOF

  cat > "$AGENTS_HOME/logs/agent.jsonl" <<'EOF'
{"ts":"2026-10-28 10:00:00","level":"info","component":"agent","event":"unrelated_info"}
{"ts":"2026-10-28 10:00:01","level":"warn","component":"agent","event":"disk_warning"}
{"ts":"2026-10-28 10:00:02","level":"info","component":"agent","event":"task_step","task":"T1"}
{"ts":"2026-10-28 10:00:03","level":"error","component":"agent","event":"crash"}
EOF

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-sync.json" ]

  [ "$(jq -r '.Logs | length' "$STUB_HOME/last-sync.json")" -eq 3 ]
  [ "$(jq -r '.[0].event' <<< "$(jq -c .Logs "$STUB_HOME/last-sync.json")")" = "disk_warning" ]
  [ "$(jq -r '.[1].event' <<< "$(jq -c .Logs "$STUB_HOME/last-sync.json")")" = "task_step" ]
  [ "$(jq -r '.[2].event' <<< "$(jq -c .Logs "$STUB_HOME/last-sync.json")")" = "crash" ]
}

@test "node sync retries on failed sync without breaking heartbeat" {
  setup_node_sync_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"

  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/tasks/T1"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<'EOF'
{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}
EOF

  cat > "$AGENTS_HOME/tasks/T1/meta.env" <<'EOF'
STATUS=running
RUN=1
JOB_ID=job-1
EOF
  printf 'hello prompt\n' > "$AGENTS_HOME/tasks/T1/prompt-1.md"

  touch "$STUB_HOME/sync-fail"

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-heartbeat.json" ]
  [ ! -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]

  # Now remove failure flag and run again: sync should retry and succeed
  rm -f "$STUB_HOME/sync-fail"
  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/last-sync.json" ]
  [ -f "$AGENTS_HOME/tasks/T1/sync_manifest.json" ]
}
