#!/usr/bin/env bats
# A state-path collision must warn and retry without accumulating files or stopping heartbeats.

load helpers

# sync_state_stub accepts sync requests and records that the heartbeat was still attempted.
sync_state_stub() {
  mv "$HOME/.local/bin/agentbus" "$HOME/.local/bin/sync-real-agentbus"
  cat > "$HOME/.local/bin/agentbus" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == node-request ]]; then
  shift
  route=/v1/heartbeat
  validation=false
  while (($#)); do
    case $1 in
      --path) route=$2; shift 2 ;;
      --validate) validation=true; shift ;;
      *) shift ;;
    esac
  done
  cat > "$STUB_HOME/request.json"
  if [[ $route == /v1/sync ]]; then
    printf '{"OK":true}\n'
  else
    if [[ $validation == false ]]; then touch "$STUB_HOME/heartbeat-attempted"; fi
    printf '{"Job":null,"Stops":[]}\n'
  fi
  exit 0
fi
exec "$HOME/.local/bin/sync-real-agentbus" "$@"
STUB
  chmod +x "$HOME/.local/bin/agentbus"
}

@test "a directory at the sync manifest path warns without accumulating temporary files" {
  sync_state_stub
  printf '\nLEAD_URL=http://127.0.0.1:7780\nINSTANCE_NAME=node-test\n' >> "$AGENTS_HOME/agents.conf"
  local task="$AGENTS_HOME/tasks/T1"
  mkdir -p "$task/sync_manifest.json" "$AGENTS_HOME/bus"
  printf '%s\n' '{"jobs":{"job-1":{"assignment":1,"task_id":"T1","launched":true,"state":"running"}},"outbox":[]}' > "$AGENTS_HOME/bus/node-jobs.json"
  printf 'STATUS=running\nRUN=1\nAGENT=solo\nJOB_ID=job-1\n' > "$task/meta.env"
  printf 'prompt\n' > "$task/prompt-1.md"

  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/heartbeat-attempted" ]
  # A failed replacement must not be mistaken for a successful move into the directory.
  jq -e 'select(.event == "node_sync_save_failed")' "$AGENTS_HOME/logs/agent.jsonl" > /dev/null
  [ -z "$(find "$task/sync_manifest.json" -mindepth 1 -print -quit)" ]
}
