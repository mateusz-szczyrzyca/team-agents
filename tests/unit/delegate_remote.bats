#!/usr/bin/env bats
# Remote delegation in two installations on one machine: the lead (the test HOME) and node2 (NODE, 127.0.0.2,
# capabilities os:linux hw:rp2350). The lead also has its own node daemon identity, lead-node (os:linux only), which
# is the requester: the member solo delegates a step that needs hw:rp2350 and the request becomes a job the lead
# hands to node2. No daemon runs in the background: every step is an explicit `agent node --once` cycle, so a
# failing assertion shows up at once. Only loopback addresses are used and no real agent CLI runs.

load helpers

# teardown — stops the lead and its stub agents, even when an assertion fails.
teardown() {
  local f pid rc
  for f in "$HOME/lead.pid"; do
    [[ -f $f ]] || continue
    pid=$(cat "$f")
    # A process already reaped by the test is an expected non-running answer (as in node_two_homes.bats).
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
    [[ -f $p ]] && kill -TERM -- -"$(cat "$p")" 2> /dev/null
  done
  return 0
}

# free_port — a free TCP port on 127.0.0.1 (the lead's listener address).
free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

# setup_node — the node2 installation under $BATS_TEST_TMPDIR/node2 (as node_two_homes.bats), declaring hw:rp2350
# and a clone of the shared remote as NODE_REPOS[repo] (the logical repository name of the tasks).
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
  echo '{"worker": {"new": "commit-push-other"}}' > "$NODE/scenario.json"
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
LEAD_URL="http://127.0.0.1:$LEAD_PORT"
NODE_POLL_SEC=1
INSTANCE_NAME=node2
INSTANCE_CAPABILITIES=(os:linux hw:rp2350)
declare -A NODE_REPOS=([repo]="$NODE/repo")
CONF
  # The test wrapper of AGENTBUS_BIN sends node-request from 127.0.0.2, so the lead identifies this device as node2.
  cat > "$NODE/.local/bin/agentbus-node2" <<WRAP
#!/usr/bin/env bash
# Test wrapper: node-request leaves from 127.0.0.2 (node2); every other subcommand is the plain agentbus.
if [[ \${1:-} == node-request ]]; then shift; exec "$NODE/.local/bin/agentbus" node-request --source 127.0.0.2 "\$@"; fi
exec "$NODE/.local/bin/agentbus" "\$@"
WRAP
  chmod +x "$NODE/.local/bin/agentbus-node2"
}

# configure_lead — the lead's configuration: it listens on 127.0.0.1, knows node2, and its own node identity
# (lead-node, os:linux only) talks to it over loopback.
configure_lead() {
  grep -q '^LEAD_LISTEN=' "$HOME/agents/agents.conf" && return 0
  cat >> "$HOME/agents/agents.conf" <<CONF
LEAD_LISTEN=127.0.0.1:$LEAD_PORT
LEAD_NODES="node2"
LEAD_URL="http://127.0.0.1:$LEAD_PORT"
INSTANCE_NAME=lead-node
INSTANCE_CAPABILITIES=(os:linux)
NODE_TTL_SEC=30
NODE_POLL_SEC=1
CONF
}

# start_lead — starts the lead (agent lead) on 127.0.0.1:$LEAD_PORT with a tailscale stub that knows node2 at 127.0.0.2.
start_lead() {
  configure_lead
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

# wait_lead_up — waits until the lead answers its API on 127.0.0.1; on failure shows the end of its log.
wait_lead_up() {
  for _ in $(seq 1 75); do
    curl -fsS --noproxy '*' --max-time 1 "http://127.0.0.1:$LEAD_PORT/v1/nodes" > /dev/null 2>&1 && return 0
    sleep 0.2
  done
  echo "the lead does not answer on 127.0.0.1:$LEAD_PORT" >&2
  tail -n 40 "$HOME/lead.log" >&2
  return 1
}

# stop_lead — stops the lead with SIGTERM and waits for it; its exit status must be 0.
stop_lead() {
  local pid rc=0
  pid=$(cat "$HOME/lead.pid")
  kill -TERM "$pid"
  wait "$pid" || rc=$?
  [[ $rc -eq 0 ]] || { echo "lead exit $rc" >&2; tail -n 40 "$HOME/lead.log" >&2; return 1; }
  rm "$HOME/lead.pid"
}

# lead_cycle — one poll of the lead's own node daemon (lead-node): submits queued remote requests as jobs and
# delivers finished helper results to the requester.
lead_cycle() {
  agent node --once > "$HOME/leadcycle.log" 2>&1 || { tail -n 40 "$HOME/leadcycle.log" >&2; return 1; }
}

# node_cycle — one poll of node2: takes jobs it can run, and reports finished tasks back to the lead.
node_cycle() {
  env HOME="$NODE" AGENTS_HOME="$NODE/agents" STUB_HOME="$NODE" STUB_UNITS="$NODE/units" \
    AGENTBUS_BIN="$NODE/.local/bin/agentbus-node2" PATH="$BATS_TEST_DIRNAME/stubs:$NODE/.local/bin:$PATH" \
    "$NODE/.local/bin/agent" node --once > "$NODE/cycle.log" 2>&1 || { tail -n 40 "$NODE/cycle.log" >&2; return 1; }
}

# wait_node_task_done — waits until the task on node2 (started for the job) has STATUS=done.
wait_node_task_done() {
  for _ in $(seq 1 300); do
    grep -qs '^STATUS=done$' "$NODE"/agents/tasks/*/meta.env && return 0
    sleep 0.2
  done
  echo "no task on node2 reached STATUS=done" >&2
  return 1
}

# lead_get path — GET from the lead's API (loopback, the lead's own CLI).
lead_get() {
  agentbus node-request --url "http://127.0.0.1:$LEAD_PORT" --path "$1" --get
}

# wait_helper_result status — waits until the member solo has received the helper result of the given status
# (done, failed). The fake member saves what its wait for a message returned (delivered-solo.txt).
wait_helper_result() {
  wait_grep "RESULT H" "$HOME/delivered-solo.txt" 120
  wait_grep "[$1]" "$HOME/delivered-solo.txt" 1
}

# member_delegate action — the member solo on the lead runs a task whose action is the given stub action.
member_delegate() {
  echo "{\"solo\": {\"new\": \"$1\"}, \"worker\": {\"new\": \"commit-push-other\"}}" > "$HOME/scenario.json"
  agent run solo "delegate a remote build step"
}

# wait_remote_file branch path — waits until the remote branch holds the file (the member's later commit is pushed).
wait_remote_file() {
  for _ in $(seq 1 150); do
    git -C "$HOME/remote.git" cat-file -e "$1:$2" 2> /dev/null && return 0
    sleep 0.2
  done
  echo "no $2 on $1 in the remote" >&2
  return 1
}

@test "a delegate with requires becomes a job, runs on the node with the capability, and the helper result returns to the member" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  member_delegate "push-delegate:hw:rp2350"
  wait_grep 'REQUEST H' "$HOME/agents/bus/messages.jsonl" 60
  lead_cycle
  [[ $(lead_get /v1/jobs | jq 'length') == 1 ]]
  local job branch
  job=$(lead_get /v1/jobs | jq -r '.[0].Request.ID')
  [[ $(lead_get "/v1/jobs/$job" | jq -r '.Request.Requester') == solo@lead-node ]]
  lead_get "/v1/jobs/$job" | jq -e '.Request.BaseSHA | length == 40' > /dev/null
  node_cycle
  wait_node_task_done
  node_cycle
  lead_cycle
  wait_helper_result done
  [[ $(lead_get "/v1/jobs/$job" | jq -r '.Node') == node2 ]]
  branch=$(lead_get "/v1/jobs/$job" | jq -r '.Branch')
  [[ $(git -C "$HOME/remote.git" show "$branch:other.txt") == other ]]
  git -C "$HOME/remote.git" merge-base --is-ancestor "$(lead_get "/v1/jobs/$job" | jq -r '.Request.BaseSHA')" "$branch"
}

@test "a delegate whose capability the requester has stays local and creates no job" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  member_delegate "push-delegate:os:linux"
  wait_grep 'REQUEST H' "$HOME/agents/bus/messages.jsonl" 60
  lead_cycle
  [[ $(lead_get /v1/jobs | jq 'length') == 0 ]]
  wait_helper_result done
}

@test "a delegate whose commit is not pushed fails its helper with NotPushed and submits no job" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  member_delegate "commit-delegate:hw:rp2350"
  wait_grep 'REQUEST H' "$HOME/agents/bus/messages.jsonl" 60
  lead_cycle
  grep -q 'NotPushed' "$HOME/agents/bus/messages.jsonl"
  [[ $(lead_get /v1/jobs | jq 'length') == 0 ]]
}

@test "a delegate made while the lead is stopped waits locally and is submitted once after the lead returns" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  stop_lead
  echo '{"solo": {"new": "push-delegate:hw:rp2350"}, "worker": {"new": "commit-push-other"}}' > "$HOME/scenario.json"
  agent run solo "delegate a remote build step"
  wait_grep 'REQUEST H' "$HOME/agents/bus/messages.jsonl" 60
  ! lead_cycle
  start_lead
  lead_cycle
  [[ $(lead_get /v1/jobs | jq 'length') == 1 ]]
  [[ $(lead_get /v1/jobs | jq -r '.[0].Request.Requester') == solo@lead-node ]]
  node_cycle
  wait_node_task_done
  node_cycle
  lead_cycle
  wait_helper_result done
}

@test "a checkout failure on the node gives the member a failed helper result with its cause, and nothing starts on the node" {
  LEAD_PORT=$(free_port)
  setup_node
  mv "$NODE/repo" "$NODE/repo.away"
  start_lead
  member_delegate "push-delegate:hw:rp2350"
  wait_grep 'REQUEST H' "$HOME/agents/bus/messages.jsonl" 60
  lead_cycle
  node_cycle
  node_cycle
  lead_cycle
  wait_helper_result failed
  local job
  job=$(lead_get /v1/jobs | jq -r '.[0].Request.ID')
  [[ $(lead_get "/v1/jobs/$job" | jq -r '.State') == failed ]]
  lead_get "/v1/jobs/$job" | jq -r '.Result' | grep -q 'CheckoutFailed\|FetchFailed'
  [[ -z $(ls -A "$NODE/agents/tasks") ]]
}

@test "the job keeps the SHA captured at the delegate call when the requester's HEAD moves on" {
  LEAD_PORT=$(free_port)
  setup_node
  start_lead
  member_delegate "push-delegate-advance:hw:rp2350"
  wait_grep 'REQUEST H' "$HOME/agents/bus/messages.jsonl" 60
  local branch
  branch=$(git -C "$HOME/remote.git" for-each-ref --format='%(refname:short)' 'refs/heads/agents/solo/*' | head -n 1)
  wait_remote_file "$branch" advance.txt
  lead_cycle
  local job captured
  job=$(lead_get /v1/jobs | jq -r '.[0].Request.ID')
  captured=$(lead_get "/v1/jobs/$job" | jq -r '.Request.BaseSHA')
  # The member pushed the captured commit first and moved its HEAD after delegating: the job names the older commit.
  [[ $captured == "$(git -C "$HOME/remote.git" rev-parse "$branch~1")" ]]
  node_cycle
  wait_node_task_done
  node_cycle
  lead_cycle
  wait_helper_result done
  local jb
  jb=$(lead_get "/v1/jobs/$job" | jq -r '.Branch')
  ! git -C "$HOME/remote.git" cat-file -e "$jb:advance.txt"
}
