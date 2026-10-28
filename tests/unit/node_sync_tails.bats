#!/usr/bin/env bats
# Partial final lines count toward the activity and error tail limits.

load helpers

# sync_tails_stub records uploaded data without requiring a real remote job.
sync_tails_stub() {
  mv "$HOME/.local/bin/agentbus" "$HOME/.local/bin/tails-real-agentbus"
  cat > "$HOME/.local/bin/agentbus" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == node-request ]]; then
  shift
  route=/v1/heartbeat
  while (($#)); do
    case $1 in
      --path) route=$2; shift 2 ;;
      *) shift ;;
    esac
  done
  if [[ $route == /v1/sync ]]; then
    cat > "$STUB_HOME/tail-sync.json"
    printf '{"OK":true}\n'
  else
    cat > "$STUB_HOME/tail-heartbeat.json"
    printf '{"Job":null,"Stops":[]}\n'
  fi
  exit 0
fi
exec "$HOME/.local/bin/tails-real-agentbus" "$@"
STUB
  chmod +x "$HOME/.local/bin/agentbus"
}

@test "node sync counts partial final lines in activity and error tails" {
  sync_tails_stub
  printf '\nLEAD_URL=http://127.0.0.1:7780\nINSTANCE_NAME=node-test\n' >> "$AGENTS_HOME/agents.conf"
  local task="$AGENTS_HOME/tasks/T1"
  mkdir -p "$task" "$AGENTS_HOME/bus"
  printf '%s\n' '{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}' > "$AGENTS_HOME/bus/node-jobs.json"
  printf 'STATUS=running\nRUN=1\nAGENT=solo\nJOB_ID=job-1\n' > "$task/meta.env"
  seq 1 2000 > "$task/activity.jsonl"
  printf 'partial activity' >> "$task/activity.jsonl"
  seq 1 500 > "$task/err-1.log"
  printf 'partial error' >> "$task/err-1.log"

  run agent node --once
  [ "$status" -eq 0 ]
  local name limit lines digest
  for name in activity.jsonl err-1.log; do
    limit=2000
    [[ $name != err-1.log ]] || limit=500
    jq -j --arg name "$name" '.Files[] | select(.Name == $name) | .Content' "$STUB_HOME/tail-sync.json" > "$STUB_HOME/content"
    lines=$(awk 'END {print NR}' "$STUB_HOME/content")
    [ "$lines" -eq "$limit" ]
    [ "$(jq -r --arg name "$name" '.Files[] | select(.Name == $name) | .Cut' "$STUB_HOME/tail-sync.json")" = true ]
    digest=$(sha256sum < "$STUB_HOME/content")
    [ "${digest%% *}" = "$(jq -r --arg name "$name" '.Files[] | select(.Name == $name) | .SHA256' "$STUB_HOME/tail-sync.json")" ]
  done
}
