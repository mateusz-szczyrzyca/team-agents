#!/usr/bin/env bats
# Regression test: node daemon heartbeat assignment persistence with maximum-size escaped task.

load helpers

# setup_large_task_bus_stub returns bounded deterministic lead replies for one assignment.
setup_large_task_bus_stub() {
  local real_bus="$HOME/.local/bin/agentbus"
  mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == node-request ]]; then
  shift; path=""; validate=0
  while (($#)); do
    case $1 in --path) path=$2; shift 2 ;; --validate) validate=1; shift ;; *) shift ;; esac
  done
  if ((validate)); then printf '{}\n'; exit 0; fi
  if [[ $path == "/v1/heartbeat" ]]; then
    if [[ ! -f "$STUB_HOME/job-delivered" ]]; then
      touch "$STUB_HOME/job-delivered"
      python3 -c 'import json; r={"ID":"job-large-1","Repository":"repo","BaseRef":"main","BaseSHA":"","Requires":[],"Selector":"solo","Task":"\""*65536,"Requester":"human"}; print(json.dumps({"Job":{"Request":r,"State":"assigned","Assignment":1,"Node":"node-test","History":[{"State":"queued","At":"2026-10-25T20:00:00.000000000Z"},{"State":"assigned","At":"2026-10-25T20:00:01.000000000Z"}]},"Stops":[]}))'
    else
      printf '{"Job":null,"Stops":[]}\n'
    fi
    exit 0
  fi
  if [[ $path == /v1/jobs/*/state ]]; then
    cat > "$STUB_HOME/last-state.json"
    sid=$(jq -r '.ID' "$STUB_HOME/last-state.json"); asgn=$(jq -r '.Assignment' "$STUB_HOME/last-state.json")
    tid=$(jq -r '.TaskID' "$STUB_HOME/last-state.json"); br=$(jq -r '.Branch' "$STUB_HOME/last-state.json")
    python3 -c 'import json, sys; sid, asgn, tid, br = sys.argv[1:5]; r={"ID":sid,"Repository":"repo","BaseRef":"main","BaseSHA":"","Requires":[],"Selector":"solo","Task":"\""*65536,"Requester":"human"}; print(json.dumps({"Request":r,"State":"running","Node":"node-test","Assignment":int(asgn),"TaskID":tid,"Branch":br,"History":[{"State":"running","At":"2026-10-25T20:00:02.000000000Z"}]}))' "$sid" "$asgn" "$tid" "$br"
    exit 0
  fi
  exit 0
fi
exec "$HOME/.local/bin/real-agentbus" "$@"
STUB
  chmod +x "$real_bus"
}

@test "maximum-size escaped task persists assignment state and prepares runner task" {
  setup_large_task_bus_stub
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$AGENTS_HOME/bus/node-jobs.json" ]
  [ "$(jq -r '.jobs["job-large-1"].assignment' "$AGENTS_HOME/bus/node-jobs.json")" = 1 ]
  jq -e '.jobs["job-large-1"].request.Task == ("\"" * 65536)' "$AGENTS_HOME/bus/node-jobs.json" >/dev/null
  local tid
  tid=$(jq -r '.jobs["job-large-1"].task_id' "$AGENTS_HOME/bus/node-jobs.json")
  [ -f "$AGENTS_HOME/tasks/$tid/meta.env" ]
  [ "$(status_of "$tid")" != failed ]
}
