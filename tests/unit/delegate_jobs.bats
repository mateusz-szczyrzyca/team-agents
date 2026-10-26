#!/usr/bin/env bats
# Two independent temporary installations exercise the real local bus, lead HTTP, git, node poll and stub runner.

load helpers

# teardown stops only the fixture lead and both installations' stub units.
teardown() {
  local f pid rc
  if [[ -f $HOME/lead.pid ]]; then
    pid=$(cat "$HOME/lead.pid")
    if kill -0 "$pid"; then
      kill -TERM "$pid"
      rc=0
      wait "$pid" || rc=$?
      if [[ $rc != 0 && $rc != 143 ]]; then echo "lead cleanup failed: $rc" >&2; fi
    fi
  fi
  for f in "$STUB_UNITS"/*.pid "$BATS_TEST_TMPDIR/node"/units/*.pid; do
    [[ -f $f ]] || continue
    pid=$(cat "$f")
    # A completed stub unit need not still have a process group to stop.
    if kill -0 -- -"$pid"; then
      if ! kill -TERM -- -"$pid"; then echo "stub cleanup failed: $pid" >&2; fi
    fi
  done
}

# installations creates node2 and configures the lead's own node with no hardware capability.
installations() {
  export AGENTBUS_BIN="$HOME/.local/bin/agentbus"
  LEAD_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.2",0)); print(s.getsockname()[1])')
  NODE="$BATS_TEST_TMPDIR/node"
  mkdir -p "$NODE/.local/bin" "$NODE/agents"/{tasks,worktrees,bus,prompts} "$NODE/units"
  install -m755 "$REPO_ROOT/bin/agent" "$NODE/.local/bin/agent"
  cp "$BUILD_DIR/agentbus" "$NODE/.local/bin/agentbus"
  cp "$BATS_TEST_DIRNAME/stubs/fake-claude" "$NODE/.local/bin/claude"
  cp "$REPO_ROOT"/prompts/*.md "$NODE/agents/prompts/"
  : > "$NODE/agents/secrets.env"
  echo '{"worker":{"new":"commit-push"}}' > "$NODE/scenario.json"
  git clone -q --branch main "$HOME/remote.git" "$NODE/repo"
  cat > "$NODE/agents/agents.conf" <<CONF
declare -A AGENT_TYPE=([worker]=claude)
declare -A AGENT_MODEL=()
NORMAL_AGENTS=()
WORKER_AGENTS=(worker)
declare -A INVESTIGATORS_TYPE=()
CLAUDE_BIN="$NODE/.local/bin/claude"
RUN_PATH="$BATS_TEST_DIRNAME/stubs:$NODE/.local/bin:/usr/local/bin:/usr/bin:/bin"
LEAD_URL=http://127.0.0.2:$LEAD_PORT
INSTANCE_NAME=node2
INSTANCE_CAPABILITIES=(os:linux hw:rp2350)
declare -A NODE_REPOS=([repo]="$NODE/repo")
CONF
  cat > "$NODE/.local/bin/node-bus" <<WRAP
#!/usr/bin/env bash
set -euo pipefail
if [[ \${1:-} == node-request ]]; then shift; exec "$NODE/.local/bin/agentbus" node-request --source 127.0.0.2 "\$@"; fi
exec "$NODE/.local/bin/agentbus" "\$@"
WRAP
  chmod +x "$NODE/.local/bin/node-bus"
  cat >> "$AGENTS_HOME/agents.conf" <<CONF
LEAD_LISTEN=127.0.0.2:$LEAD_PORT
LEAD_NODES=node2
LEAD_URL=http://127.0.0.1:$LEAD_PORT
INSTANCE_NAME=lead
INSTANCE_CAPABILITIES=(os:linux)
CONF
}

# lead_start starts the isolated lead with a fail-closed whois fixture for node2.
lead_start() {
  mkdir -p "$HOME/tsbin"
  cat > "$HOME/tsbin/tailscale" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ ${1:-} == whois && ${2:-} == --json && ${3:-} == 127.0.0.2 ]] || exit 2
printf '%s\n' '{"Node":{"Name":"node2.example.com.","ComputedName":"node2"}}'
STUB
  chmod +x "$HOME/tsbin/tailscale"
  mkdir -p "$AGENTS_HOME/logs"
  : > "$AGENTS_HOME/logs/agentbus.jsonl"
  PATH="$HOME/tsbin:$PATH" "$HOME/.local/bin/agent" lead > "$HOME/lead.log" 2>&1 &
  echo "$!" > "$HOME/lead.pid"
  wait_grep 'lead_listening' "$AGENTS_HOME/logs/agentbus.jsonl" 10
}

# lead_stop gracefully stops this test's lead and retains its registry on disk.
lead_stop() {
  local pid
  pid=$(cat "$HOME/lead.pid")
  kill -TERM "$pid"
  wait "$pid"
  rm "$HOME/lead.pid"
}

# source_cycle executes the requesting node's ordinary recovery and poll path once.
source_cycle() { "$HOME/.local/bin/agent" node --once; }

# destination_cycle executes node2 with independent HOME, metadata, stub processes and transport identity.
destination_cycle() {
  env HOME="$NODE" AGENTS_HOME="$NODE/agents" STUB_HOME="$NODE" STUB_UNITS="$NODE/units" \
    AGENTBUS_BIN="$NODE/.local/bin/node-bus" PATH="$BATS_TEST_DIRNAME/stubs:$NODE/.local/bin:$PATH" \
    "$NODE/.local/bin/agent" node --once
}

# jobs reads only the lead's public job view.
jobs() { "$AGENTBUS_BIN" node-request --url "http://127.0.0.1:$LEAD_PORT" --get --path /v1/jobs; }

# helper reads the requester's durable local helper record.
helper() { "$AGENTBUS_BIN" helper-get --dir "$AGENTS_HOME/bus" --id 1; }

# request_step starts the fake requesting agent and waits until its commit and delegation are complete.
request_step() {
  scenario "{\"solo\":{\"new\":\"$1\"},\"worker\":{\"new\":\"commit-push\"}}"
  "$HOME/.local/bin/agent" run solo "delegate a build"
  SOURCE_TASK=$(task_of solo)
  wait_status "$SOURCE_TASK" "done"
  [ "$(helper | jq -r .base_sha)" = "$(cat "$HOME/captured-sha.txt")" ]
}

# read_result uses the requester's actual MCP configuration to receive the ordinary local bus message.
read_result() {
  python3 - "$AGENTS_HOME/tasks/$SOURCE_TASK/mcp.json" <<'PY'
import json, subprocess, sys
cfg = json.load(open(sys.argv[1]))["mcpServers"]["agentbus"]
request = json.dumps({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})+"\n"
request += json.dumps({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_messages","arguments":{}}})+"\n"
out = subprocess.run([cfg["command"]]+cfg["args"], input=request, text=True, capture_output=True, check=True).stdout
print(json.loads(out.splitlines()[-1])["result"]["content"][0]["text"])
PY
}

# complete_job runs the capable node, waits for its stub agent, reports completion and delivers the source result.
complete_job() {
  destination_cycle
  local tid
  tid=$(jq -r '.jobs[] | .task_id' "$NODE/agents/bus/node-jobs.json")
  ( AGENTS_HOME="$NODE/agents" wait_status "$tid" "done" 20 )
  destination_cycle
  source_cycle
  [ "$(helper | jq -r .status)" = "done" ]
  jobs | jq -e '.[0] | .State == "done" and .Node == "node2" and (.Branch | length > 0)' >/dev/null
  read_result > "$HOME/result.txt"
  grep -q 'RESULT H1 \[done\]' "$HOME/result.txt"
  grep -q node2 "$HOME/result.txt"
}

@test "delegated job uses the pushed SHA, capable node, git branch and ordinary requester message" {
  installations; lead_start
  request_step round8-delegate:hw:rp2350
  source_cycle
  [ "$(jobs | jq length)" = 1 ]
  jobs | jq -e --arg sha "$(cat "$HOME/captured-sha.txt")" '.[0].Request | .BaseSHA == $sha and .Requester == "solo@lead" and .Selector == "worker"' >/dev/null
  complete_job
  local branch
  branch=$(jobs | jq -r '.[0].Branch')
  [ "$(git -C "$HOME/remote.git" show "$branch:hello.txt")" = hello ]
  git -C "$HOME/remote.git" merge-base --is-ancestor "$(cat "$HOME/captured-sha.txt")" "$branch"
}

@test "locally satisfied requirements keep the existing local helper flow" {
  installations; lead_start
  request_step round8-delegate:os:linux
  "$HOME/.local/bin/agent" _dispatch-helpers
  [ "$(helper | jq -r .status)" != queued ]
  local tid
  tid=$(helper | jq -r .helper_task)
  wait_status "$tid" "done"
  wait_grep 'RESULT H1' "$AGENTS_HOME/bus/messages.jsonl" 10
  [ "$(jobs | jq length)" = 0 ]
  read_result | grep -q 'RESULT H1 \[done\]'
}

@test "an unpushed SHA fails with a push instruction before creating any job" {
  installations; lead_start
  request_step round8-unpushed:hw:rp2350
  source_cycle
  [ "$(helper | jq -r .status)" = failed ]
  helper | jq -r .summary | grep -q NotPushed
  helper | jq -r .summary | grep -q 'push'
  [ "$(jobs | jq length)" = 0 ]
}

@test "lead outage retains one frozen submission across requesting node restarts" {
  installations; lead_start; lead_stop
  request_step round8-delegate:hw:rp2350
  source_cycle
  helper > "$HOME/pending.json"
  jq -e '.status == "queued" and (.remote_job.ID | length > 0)' "$HOME/pending.json" >/dev/null
  source_cycle
  [ "$(helper | jq -c .remote_job)" = "$(jq -c .remote_job "$HOME/pending.json")" ]
  lead_start; source_cycle
  [ "$(jobs | jq length)" = 1 ]
  [ "$(jobs | jq -r '.[0].Request.ID')" = "$(jq -r .remote_job.ID "$HOME/pending.json")" ]
  complete_job
}

@test "failed destination checkout returns a failed helper result without starting a task" {
  installations; lead_start
  mv "$NODE/repo" "$NODE/repo-away"
  request_step round8-delegate:hw:rp2350
  source_cycle
  [ "$(jobs | jq length)" = 1 ]
  destination_cycle
  destination_cycle # A later heartbeat reports the provisioning failure recorded by the first poll.
  source_cycle
  [ "$(helper | jq -r .status)" = failed ]
  helper | jq -r .summary | grep -q FetchFailed
  [ "$(find "$NODE/agents/tasks" -name meta.env | wc -l)" = 0 ]
  read_result | grep -q 'RESULT H1 \[failed\]'
}

@test "requester commits after delegation cannot change the submitted base" {
  installations; lead_start
  request_step round8-advance:hw:rp2350
  source_cycle
  [ "$(jobs | jq length)" = 1 ]
  [ "$(jobs | jq -r '.[0].Request.BaseSHA')" = "$(cat "$HOME/captured-sha.txt")" ]
  complete_job
  local branch rc=0
  branch=$(jobs | jq -r '.[0].Branch')
  git -C "$HOME/remote.git" cat-file -e "$branch:advance.txt" || rc=$?
  [ "$rc" = 128 ] # Missing file proves the newer requester commit was not included.
}
