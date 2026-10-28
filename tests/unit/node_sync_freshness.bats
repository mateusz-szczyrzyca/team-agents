#!/usr/bin/env bats
# Tests for review finding M3: snapshot freshness boundary race during node sync.

load helpers

# setup_bus_stub intercepts node-request calls to /v1/heartbeat and /v1/sync.
setup_bus_stub() {
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
    count=0
    [[ -f "$STUB_HOME/sync-count" ]] && count=$(< "$STUB_HOME/sync-count")
    count=$((count + 1))
    echo "$count" > "$STUB_HOME/sync-count"
    cat > "$STUB_HOME/sync-$count.json"
    if [[ -f "$STUB_HOME/pre-ack-mutate.sh" ]]; then
      bash "$STUB_HOME/pre-ack-mutate.sh" "$count"
    fi
    printf '{"OK":true}\n'
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"
}

# setup_test_env initializes configuration, bus state, and task directory for a job task.
setup_test_env() {
  local tid=$1 jid=$2
  setup_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  mkdir -p "$AGENTS_HOME/bus" "$AGENTS_HOME/logs" "$AGENTS_HOME/tasks/$tid"
  cat > "$AGENTS_HOME/bus/node-jobs.json" <<EOF
{"jobs":{"$jid":{"assignment":1,"task_id":"$tid","launched":true,"state":"done"}},"outbox":[]}
EOF
  printf 'STATUS=done\nTYPE=claude\nMODEL=test\n' > "$AGENTS_HOME/tasks/$tid/meta.env"
  unset AGENT
}

@test "M3: concurrent update before sync acknowledgement is uploaded on next poll" {
  setup_test_env T1 job-1
  echo '{"ts":"2026-10-28T11:59:59","level":"info","event":"running"}' > "$AGENTS_HOME/tasks/T1/activity.jsonl"
  cat > "$STUB_HOME/pre-ack-mutate.sh" <<'EOF'
if (( $1 == 1 )); then
  echo "FINISHED=2026-10-28T12:00:00" >> "$AGENTS_HOME/tasks/T1/meta.env"
  echo '{"ts":"2026-10-28T12:00:00","level":"info","event":"finished"}' >> "$AGENTS_HOME/tasks/T1/activity.jsonl"
fi
EOF

  agent node --once
  [ "$(< "$STUB_HOME/sync-count")" -eq 1 ]
  [ -z "$(jq -r '.Metadata.FINISHED // empty' "$STUB_HOME/sync-1.json")" ]

  # Poll 2 must not fast-skip; pre-ACK mutation (both FINISHED and final activity) must be uploaded
  agent node --once
  [ "$(< "$STUB_HOME/sync-count")" -eq 2 ]
  [ "$(jq -r '.Metadata.FINISHED // empty' "$STUB_HOME/sync-2.json")" = "2026-10-28T12:00:00" ]
  activity_content=$(jq -r '.Files[] | select(.Name == "activity.jsonl") | .Content' "$STUB_HOME/sync-2.json")
  [[ "$activity_content" == *"finished"* ]]

  # Poll 3 fast-skips unchanged terminal task
  agent node --once
  [ "$(< "$STUB_HOME/sync-count")" -eq 2 ]
}

@test "M3: concurrent mutation during clean legacy promotion is uploaded on next poll" {
  setup_test_env T2 job-2
  cat > "$AGENTS_HOME/tasks/T2/sync_manifest.json" <<'EOF'
{"metadata":{"STATUS":"done","TYPE":"claude","MODEL":"test","JOB_ID":"job-2","RESTARTS":"0"},"files":{}}
EOF

  cat > "$HOME/.local/bin/mktemp" <<'SPY'
#!/usr/bin/env bash
if [[ ${1:-} == *"sync_manifest.json.tmp"* ]] && [[ -f "$STUB_HOME/mutate-on-manifest-mktemp" ]]; then
  echo "late artifact" > "$AGENTS_HOME/tasks/T2/final.md"
  rm -f "$STUB_HOME/mutate-on-manifest-mktemp"
fi
exec /usr/bin/mktemp "$@"
SPY
  chmod +x "$HOME/.local/bin/mktemp"
  touch "$STUB_HOME/mutate-on-manifest-mktemp"

  # Poll 1 promotes manifest without sending sync
  agent node --once
  [ ! -f "$STUB_HOME/sync-count" ]
  [ "$(jq -r '.complete // false' "$AGENTS_HOME/tasks/T2/sync_manifest.json")" = "true" ]

  # Poll 2 must not fast-skip; artifact created during promotion must be uploaded
  agent node --once
  [ "$(< "$STUB_HOME/sync-count")" -eq 1 ]
  [ "$(jq -r '.Files[0].Name // empty' "$STUB_HOME/sync-1.json")" = "final.md" ]

  # Poll 3 fast-skips
  agent node --once
  [ "$(< "$STUB_HOME/sync-count")" -eq 1 ]
}

@test "M4: ambient runner environment variables do not leak into task metadata" {
  setup_test_env T3 job-3
  cat > "$AGENTS_HOME/tasks/T3/sync_manifest.json" <<'EOF'
{"metadata":{"STATUS":"done","TYPE":"claude","MODEL":"test","JOB_ID":"job-3","RESTARTS":"0"},"files":{}}
EOF

  AGENT=1 STATUS=ambient TYPE=ambient agent node --once
  [ ! -f "$STUB_HOME/sync-count" ]
  [ "$(jq -r '.complete // false' "$AGENTS_HOME/tasks/T3/sync_manifest.json")" = "true" ]
}

