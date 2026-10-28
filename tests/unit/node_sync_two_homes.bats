#!/usr/bin/env bats
# Loopback integration test for node-to-lead task and log mirroring (POST /v1/sync) across two isolated homes.

load helpers

# teardown — stops the node and the lead even when an assertion fails, and the stub agents of both installations.
teardown() {
  local f pid rc
  for f in "$BATS_TEST_TMPDIR/node2/node.pid" "$HOME/lead.pid"; do
    [[ -f $f ]] || continue
    pid=$(cat "$f")
    # A process already reaped by the test is an expected non-running answer (as in lead.bats).
    if kill -0 "$pid" 2> /dev/null; then
      kill -TERM "$pid"
      rc=0
      wait "$pid" || rc=$?
      # 143 is the expected status of a process stopped by SIGTERM; anything else is a cleanup failure to show.
      if [[ $rc -ne 0 && $rc -ne 143 ]]; then
        echo "cleanup of $f: exit $rc" >&2
      fi
    fi
  done
  for p in "$BATS_TEST_TMPDIR"/node2/units/*.pid "$STUB_UNITS"/*.pid; do
    [[ -f $p ]] || continue
    pid=$(cat "$p")
    # A stub unit that already finished before teardown is an expected non-running answer ("no such process").
    if kill -0 "$pid" 2> /dev/null; then
      rc=0
      kill -TERM -- -"$pid" 2> /dev/null || rc=$?
      if [[ $rc -ne 0 ]] && kill -0 "$pid" 2> /dev/null; then
        echo "cleanup of $p: exit $rc" >&2
      fi
    fi
  done
  # A stopped run writes its state on the way out: wait for it so removal of the test's files does not race it.
  for p in "$BATS_TEST_TMPDIR"/node2/units/*.pid "$STUB_UNITS"/*.pid; do
    [[ -f $p ]] || continue
    for _ in $(seq 1 50); do
      kill -0 "$(cat "$p")" 2> /dev/null || break   # gone: "no such process" is the answer waited for
      sleep 0.1
    done
  done
  return 0
}

# free_port — a free TCP port on 127.0.0.2 (the lead's address).
free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.2", 0)); print(s.getsockname()[1])'
}

# setup_node — creates the isolated node2 installation under $BATS_TEST_TMPDIR/node2.
setup_node() {
  NODE="$BATS_TEST_TMPDIR/node2"
  mkdir -p "$NODE/.local/bin" "$NODE/agents"/{tasks,worktrees,bus,prompts} "$NODE/units"
  install -m755 "$REPO_ROOT/bin/agent" "$NODE/.local/bin/agent"
  cp "$BUILD_DIR/agentbus" "$NODE/.local/bin/agentbus"
  cp "$BATS_TEST_DIRNAME/stubs/fake-claude" "$NODE/.local/bin/claude"
  cp "$REPO_ROOT"/prompts/*.md "$NODE/agents/prompts/"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$NODE/notify.sh"
  chmod +x "$NODE/notify.sh"
  : > "$NODE/agents/secrets.env"
  echo '{}' > "$NODE/scenario.json"
  git clone -q "$HOME/remote.git" "$NODE/repo"
  git -C "$NODE/repo" config user.name test
  git -C "$NODE/repo" config user.email test@example.com
  cat > "$NODE/agents/agents.conf" <<CONF
declare -A AGENT_TYPE=([solo]=claude [worker]=claude)
declare -A AGENT_MODEL=()
NORMAL_AGENTS=(solo)
WORKER_AGENTS=(worker)
declare -A INVESTIGATORS_TYPE=()
CLAUDE_BIN="$NODE/.local/bin/claude"
RUN_PATH="$BATS_TEST_DIRNAME/stubs:$NODE/.local/bin:/usr/local/bin:/usr/bin:/bin"
NOTIFY_CMD="$NODE/notify.sh"
AGENT_HANG_SECS=600
AGENT_HANG_POLL=1
LEAD_URL="http://127.0.0.2:$LEAD_PORT"
NODE_POLL_SEC=1
INSTANCE_NAME=node2
INSTANCE_CAPABILITIES=(os:linux hw:rp2350)
declare -A NODE_REPOS=([team-agents]="$NODE/repo")
CONF
}

# start_lead — starts the lead on 127.0.0.2:$LEAD_PORT with a tailscale stub identifying 127.0.0.2 as node2.
start_lead() {
  if ! grep -q '^LEAD_LISTEN=' "$AGENTS_HOME/agents.conf"; then
    cat >> "$AGENTS_HOME/agents.conf" <<CONF
LEAD_LISTEN=127.0.0.2:$LEAD_PORT
LEAD_NODES="node2"
INSTANCE_NAME=lead
NODE_TTL_SEC=30
CONF
  fi
  mkdir -p "$BATS_TEST_TMPDIR/tsbin"
  cat > "$BATS_TEST_TMPDIR/tsbin/tailscale" <<'STUB'
#!/usr/bin/env bash
[[ ${1:-} == whois && ${2:-} == --json ]] || { echo "tailscale stub: unsupported: $*" >&2; exit 2; }
case ${3:-} in
  127.0.0.2)
    printf '%s\n' '{"Node": {"Name": "node2.example-tailnet.ts.net.", "ComputedName": "node2", "Tags": ["tag:nodes"]}, "UserProfile": {"LoginName": "tagged-devices"}}'
    ;;
  *) echo "no match for IP: ${3:-}" >&2; exit 1 ;;
esac
STUB
  chmod +x "$BATS_TEST_TMPDIR/tsbin/tailscale"
  PATH="$BATS_TEST_TMPDIR/tsbin:$PATH" agent lead > "$HOME/lead.log" 2>&1 &
  echo "$!" > "$HOME/lead.pid"
  for _ in $(seq 1 75); do
    curl -fsS --noproxy '*' --max-time 1 "http://127.0.0.2:$LEAD_PORT/v1/nodes" > /dev/null && return 0
    sleep 0.2
  done
  echo "the lead does not answer on 127.0.0.2:$LEAD_PORT" >&2
  tail -n 40 "$HOME/lead.log" >&2
  return 1
}

# start_node — starts the node daemon (agent node) bound to source address 127.0.0.2.
start_node() {
  cat > "$NODE/.local/bin/agentbus-node2" <<WRAP
#!/usr/bin/env bash
if [[ \${1:-} == node-request ]]; then shift; exec "$NODE/.local/bin/agentbus" node-request --source 127.0.0.2 "\$@"; fi
if [[ \${1:-} == serve && " \$* " == *" --relay "* ]]; then shift; exec "$NODE/.local/bin/agentbus" serve --source 127.0.0.2 "\$@"; fi
if [[ \${1:-} == node-doorbell ]]; then shift; exec "$NODE/.local/bin/agentbus" node-doorbell --source 127.0.0.2 "\$@"; fi
exec "$NODE/.local/bin/agentbus" "\$@"
WRAP
  chmod +x "$NODE/.local/bin/agentbus-node2"
  env HOME="$NODE" AGENTS_HOME="$NODE/agents" STUB_HOME="$NODE" STUB_UNITS="$NODE/units" \
    AGENTBUS_BIN="$NODE/.local/bin/agentbus-node2" PATH="$BATS_TEST_DIRNAME/stubs:$NODE/.local/bin:$PATH" \
    "$NODE/.local/bin/agent" node > "$NODE/node.log" 2>&1 &
  echo "$!" > "$NODE/node.pid"
  wait_grep '"event":"node_heartbeat_accepted"' "$NODE/agents/logs/agent.jsonl" 20
}

# lead_post path json — POST a JSON body to the lead's API from the lead's CLI.
lead_post() {
  printf '%s' "$2" | agentbus node-request --url "http://127.0.0.2:$LEAD_PORT" --path "$1"
}

# lead_get path — GET from the lead's API.
lead_get() {
  agentbus node-request --url "http://127.0.0.2:$LEAD_PORT" --path "$1" --get
}

# job_field id field — reads one field of a job from the lead.
job_field() {
  lead_get "/v1/jobs/$1" | jq -r --arg f "$2" '.[$f]'
}

# wait_job id regex [seconds] — waits until the lead shows the job in a state matching regex.
wait_job() {
  local st=""
  for _ in $(seq 1 $((${3:-60} * 5))); do
    st=$(job_field "$1" State)
    [[ $st =~ ^($2)$ ]] && return 0
    sleep 0.2
  done
  echo "job $1 is '$st', expected /$2/" >&2
  return 1
}

@test "a node mirrors its job task metadata, files and logs to the lead under bus/remote/<node>/<task-id>" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  start_node
  echo '{"solo": {"new": "commit-push"}}' > "$NODE/scenario.json"
  lead_post /v1/jobs "$(jq -nc '{ID: "j-sync", Repository: "team-agents", BaseRef: "main", Requires: ["hw:rp2350"], Selector: "solo", Task: "add hello.txt", Requester: "human"}')" > /dev/null
  wait_job j-sync 'done' 60

  local task_id mirror
  task_id=$(job_field j-sync TaskID)
  [[ -n $task_id && $task_id != null ]]
  [[ -d "$NODE/agents/tasks/$task_id" ]]
  mirror="$AGENTS_HOME/bus/remote/node2/$task_id"

  for _ in $(seq 1 40); do
    if [[ -f "$mirror/metadata.json" && -f "$mirror/final.md" ]] && [[ $(jq -r '.STATUS // empty' "$mirror/metadata.json") == "done" ]]; then
      break
    fi
    sleep 0.2
  done

  [[ -f "$mirror/metadata.json" ]]
  [[ $(jq -r '.STATUS' "$mirror/metadata.json") == "done" ]]
  [[ $(jq -r '.JOB_ID' "$mirror/metadata.json") == "j-sync" ]]
  [[ $(jq -r '.RESTARTS' "$mirror/metadata.json") == "0" ]]
  [[ $(jq -r '.AGENT' "$mirror/metadata.json") == "solo" ]]
  jq -e 'has("SESSION_ID") | not' "$mirror/metadata.json" > /dev/null
  [[ -s "$mirror/prompt-1.md" ]]
  [[ -s "$mirror/activity.jsonl" ]]
  [[ -s "$mirror/final.md" ]]
  grep -q 'STATUS: DONE' "$mirror/final.md"
  [[ -s "$AGENTS_HOME/bus/remote/node2/logs.jsonl" ]]
  grep -q "\"task\":\"$task_id\"" "$AGENTS_HOME/bus/remote/node2/logs.jsonl"
}
