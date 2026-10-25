#!/usr/bin/env bats
# Two installations on one machine: the lead (the test HOME, listening on 127.0.0.2) and a node (NODE_HOME, named
# node2). The node connects from 127.0.0.2 through a test wrapper of AGENTBUS_BIN (node-request only), and a
# tailscale stub answers that address as the device node2, so the lead identifies it as a node. The agents are the
# fake-claude stub; nothing reaches the network beyond loopback and no real agent CLI runs.

load helpers

# teardown — stops the node and the lead even when an assertion fails, and the stub agents (as helpers.bash does).
teardown() {
  local f pid
  for f in "$BATS_TEST_TMPDIR/node2/node.pid" "$HOME/lead.pid"; do
    [[ -f $f ]] || continue
    pid=$(cat "$f")
    # A process already reaped by the test is an expected non-running answer (as in lead.bats).
    if kill -0 "$pid" 2> /dev/null; then
      kill -TERM "$pid"
      local rc=0
      wait "$pid" || rc=$?   # the exit status of a stopped daemon is not checked here
    fi
  done
  for p in "$STUB_UNITS"/*.pid; do
    [[ -f $p ]] && kill -TERM -- -"$(cat "$p")" 2>/dev/null || true
  done
}

# free_port — a free TCP port on 127.0.0.2 (the lead's address).
free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.2", 0)); print(s.getsockname()[1])'
}

# setup_node — creates the node installation under $BATS_TEST_TMPDIR/node2: its own HOME, agent and agentbus, a clone
# of the shared remote as NODE_REPOS[team-agents], and agents.conf declaring the capability hw:rp2350.
setup_node() {
  NODE="$BATS_TEST_TMPDIR/node2"
  mkdir -p "$NODE/.local/bin" "$NODE/agents"/{tasks,worktrees,bus,prompts}
  install -m755 "$REPO_ROOT/bin/agent" "$NODE/.local/bin/agent"
  cp "$BUILD_DIR/agentbus" "$NODE/.local/bin/agentbus"
  cp "$BATS_TEST_DIRNAME/stubs/fake-claude" "$NODE/.local/bin/claude"
  cp "$REPO_ROOT"/prompts/*.md "$NODE/agents/prompts/"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$NODE/notify.sh"
  chmod +x "$NODE/notify.sh"
  : > "$NODE/agents/secrets.env"
  echo '{}' > "$NODE/scenario.json"
  git clone -q "$HOME/remote.git" "$NODE/repo"
  git -C "$NODE/repo" config user.name t
  git -C "$NODE/repo" config user.email t@t
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
declare -A NODE_REPOS=([team-agents]="$NODE/repo" [broken]="$NODE/missing")
CONF
}

# start_lead — starts the lead (agent lead) on 127.0.0.2:$LEAD_PORT, with the tailscale stub that knows node2.
start_lead() {
  if ! grep -q '^LEAD_LISTEN=' "$AGENTS_HOME/agents.conf"; then   # written once; a restarted lead keeps its configuration
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
# Test stub for `tailscale whois --json <address>`: 127.0.0.2 is the device node2 (tag:nodes); anything else is unknown.
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
  wait_lead_up
}

# wait_lead_up — waits until the lead answers its API on 127.0.0.2.
wait_lead_up() {
  for _ in $(seq 1 75); do
    curl -fsS --noproxy '*' --max-time 1 "http://127.0.0.2:$LEAD_PORT/v1/nodes" > /dev/null && return 0
    sleep 0.2
  done
  echo "the lead does not answer on 127.0.0.2:$LEAD_PORT" >&2
  cat "$HOME/lead.log" >&2
  return 1
}

# stop_lead — stops the lead with SIGTERM and waits for it.
stop_lead() {
  local pid rc=0
  pid=$(cat "$HOME/lead.pid")
  kill -TERM "$pid"
  wait "$pid" || rc=$?
  [[ $rc -eq 0 ]] || { echo "lead exit $rc" >&2; return 1; }
  rm "$HOME/lead.pid"
}

# start_node — starts the node daemon (agent node) in NODE_HOME, through a wrapper of AGENTBUS_BIN that sends
# node-request from 127.0.0.2 (node2); the wrapper is test-only and changes no other subcommand.
start_node() {
  cat > "$NODE/.local/bin/agentbus-node2" <<WRAP
#!/usr/bin/env bash
# Test wrapper: node-request leaves from 127.0.0.2, so the lead identifies this device as node2.
if [[ \${1:-} == node-request ]]; then shift; exec "$NODE/.local/bin/agentbus" node-request --source 127.0.0.2 "\$@"; fi
exec "$NODE/.local/bin/agentbus" "\$@"
WRAP
  chmod +x "$NODE/.local/bin/agentbus-node2"
  HOME="$NODE" AGENTBUS_BIN="$NODE/.local/bin/agentbus-node2" \
    PATH="$BATS_TEST_DIRNAME/stubs:$NODE/.local/bin:$PATH" agent node > "$NODE/node.log" 2>&1 &
  echo "$!" > "$NODE/node.pid"
  wait_grep '"event":"node_heartbeat_accepted"' "$NODE/agents/logs/agentbus.jsonl" 20
}

# lead_post path json — POST a JSON body to the lead's API from the lead's own CLI (loopback).
lead_post() {
  printf '%s' "$2" | agentbus node-request --url "http://127.0.0.2:$LEAD_PORT" --path "$1"
}

# lead_get path — GET from the lead's API.
lead_get() {
  agentbus node-request --url "http://127.0.0.2:$LEAD_PORT" --path "$1" --get
}

# submit_job id repository requires-json selector — submits a job as the human (the lead's own CLI).
submit_job() {
  lead_post /v1/jobs "$(jq -nc --arg id "$1" --arg repo "$2" --argjson req "$3" --arg sel "$4" '
    {ID: $id, Repository: $repo, BaseRef: "main", Requires: $req, Selector: $sel,
     Task: "add hello.txt", Requester: "human"}')" > /dev/null
}

# wait_job id regex [seconds] — waits until the lead shows the job in a state matching the regex.
wait_job() {
  local st=""
  for _ in $(seq 1 $((${3:-60} * 5))); do
    st=$(lead_get "/v1/jobs/$1" | jq -r '.State')
    [[ $st =~ ^($2)$ ]] && return 0
    sleep 0.2
  done
  echo "job $1 is '$st', expected /$2/" >&2
  return 1
}

# job_field id field — one field of the lead's view of a job.
job_field() {
  lead_get "/v1/jobs/$1" | jq -r --arg f "$2" '.[$f]'
}

@test "a job with requires is taken by the node that declares the capability and its commit is reported" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  start_node
  echo '{"solo": {"new": "commit"}}' > "$NODE/scenario.json"
  submit_job j-hw team-agents '["hw:rp2350"]' solo
  wait_job j-hw 'done' 60
  [[ $(job_field j-hw Node) == node2 ]]
  local branch
  branch=$(job_field j-hw Branch)
  [[ -n $branch ]]
  git -C "$HOME/remote.git" rev-parse --verify -q "refs/heads/$branch" > /dev/null
  [[ -n $(job_field j-hw Result) ]]
}

@test "a job whose required capability the node lacks stays queued and says why" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  start_node
  submit_job j-arm team-agents '["arch:arm64"]' solo
  wait_job j-arm 'queued' 20
  [[ $(job_field j-arm WaitReason) == *"arch:arm64"* ]]
}

@test "a lead stopped while a job runs: the node finishes it and reports after the lead returns" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  start_node
  echo '{"solo": {"new": "sleep:6"}}' > "$NODE/scenario.json"
  submit_job j-outage team-agents '["hw:rp2350"]' solo
  wait_job j-outage 'running' 60
  stop_lead
  wait_grep 'STATUS=done' "$(ls -d "$NODE"/agents/tasks/*-solo)/meta.env" 60
  start_lead
  wait_job j-outage 'done' 60
  [[ -n $(job_field j-outage Result) ]]
}

@test "a job whose checkout fails is failed with its reason and nothing is recorded as started" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  start_node
  echo '{"solo": {"new": "commit"}}' > "$NODE/scenario.json"
  submit_job j-broken broken '["hw:rp2350"]' solo
  wait_job j-broken 'failed' 60
  [[ -n $(job_field j-broken Result) ]]
  [[ $(job_field j-broken History) != *'"running"'* ]]
  grep -qF '"event":"node_checkout_failed"' "$NODE/agents/logs/agentbus.jsonl"
  [[ -z $(ls -A "$NODE/agents/tasks") ]]
}

@test "a job naming a repository that is not in NODE_REPOS is refused before any work" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  start_node
  submit_job j-unknown no-such-repo '["hw:rp2350"]' solo
  wait_job j-unknown 'failed' 60
  wait_jq 'select(.event == "node_checkout_failed" and .job == "j-unknown")' "$NODE/agents/logs/agentbus.jsonl" 20
  [[ -z $(ls -A "$NODE/agents/tasks") ]]
}
