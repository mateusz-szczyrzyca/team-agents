#!/usr/bin/env bats
# Tests for node daemon remote delegation hooks (dispatch_helpers and node_poll helper handling).

load helpers

# stub_agentbus_remote intercepts helper-remote-* and node-request commands
stub_agentbus_remote() {
  local real_bus="$HOME/.local/bin/agentbus"
  if [[ ! -f "$HOME/.local/bin/real-agentbus" ]]; then
    mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  fi
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
cmd=${1:-}
case "$cmd" in
  helper-set)
    echo "helper-set: $*" >> "$STUB_HOME/helper-set.log"
    exit 0
    ;;
  helper-remote-list)
    if [[ -f "$STUB_HOME/remote-helpers.json" ]]; then
      cat "$STUB_HOME/remote-helpers.json"
    else
      echo "[]"
    fi
    exit 0
    ;;
  helper-remote-prepare)
    shift
    hid=""
    inst=""
    while (($#)); do
      case $1 in
        --id) hid=$2; shift 2 ;;
        --instance) inst=$2; shift 2 ;;
        *) shift ;;
      esac
    done
    echo "prepare: id=$hid inst=$inst" >> "$STUB_HOME/remote-prepare.log"
    if [[ -f "$STUB_HOME/prepare-storage-err" ]]; then
      exit 8
    fi
    if [[ -f "$STUB_HOME/prepare-fail" ]]; then
      cat "$STUB_HOME/prepare-fail" >&2
      exit 10
    fi
    cat <<EOF
{"ID":"$inst-h$hid","Repository":"repo","BaseRef":"main","BaseSHA":"0123456789abcdef0123456789abcdef01234567","Requires":["hw:rp2350"],"Selector":"worker","Task":"test task","Requester":"member@$inst"}
EOF
    exit 0
    ;;
  helper-remote-finish)
    echo "finish: $*" >> "$STUB_HOME/remote-finish.log"
    exit 0
    ;;
  node-request)
    shift
    path=""
    validate=0
    is_get=0
    while (($#)); do
      case $1 in
        --path) path=$2; shift 2 ;;
        --validate) validate=1; shift ;;
        --get) is_get=1; shift ;;
        *) shift ;;
      esac
    done
    if ((validate)); then printf '{}\n'; exit 0; fi
    if [[ $path == "/v1/heartbeat" ]]; then
      printf '{"Job":null,"Stops":[]}\n'
      exit 0
    fi
    if [[ $path == "/v1/jobs" ]]; then
      cat > "$STUB_HOME/last-job-submit.json"
      if [[ -f "$STUB_HOME/submit-fail" ]]; then
        exit 1
      fi
      if [[ -f "$STUB_HOME/post-mismatch" ]]; then
        jq -c '{Request: (. + {BaseSHA:"ffffffffffffffffffffffffffffffffffffffff"}),State:"queued"}' "$STUB_HOME/last-job-submit.json"
        exit 0
      fi
      printf '{"Request":%s,"State":"queued"}\n' "$(<"$STUB_HOME/last-job-submit.json")"
      exit 0
    fi
    if [[ $path == /v1/jobs/* ]]; then
      if ((is_get)); then
        if [[ -f "$STUB_HOME/get-transport-fail" ]]; then
          exit 1
        fi
        if [[ -f "$STUB_HOME/job-get.json" ]]; then
          cat "$STUB_HOME/job-get.json"
        else
          printf '{"State":"running"}\n'
        fi
        exit 0
      fi
    fi
    exit 0
    ;;
  *)
    exec "$HOME/.local/bin/real-agentbus" "$@"
    ;;
esac
STUB
  chmod +x "$real_bus"
}

@test "dispatch_helpers passes capabilities to helper-next" {
  stub_agentbus_remote
  echo 'INSTANCE_CAPABILITIES=(os:linux hw:rp2350)' >> "$AGENTS_HOME/agents.conf"
  local bus_bin="$HOME/.local/bin/real-agentbus"
  cat > "$bus_bin" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == helper-next ]]; then
  echo "helper-next: $*" >> "$STUB_HOME/helper-next.log"
  exit 0
fi
exit 0
STUB
  chmod +x "$bus_bin"

  run agent _dispatch-helpers
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/helper-next.log" ]
  grep -q -- '--capabilities' "$STUB_HOME/helper-next.log"
  grep -q 'os:linux' "$STUB_HOME/helper-next.log"
  grep -q 'hw:rp2350' "$STUB_HOME/helper-next.log"
}

@test "dispatch_helpers fails queued remote helpers when LEAD_URL is unset" {
  stub_agentbus_remote
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":1,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  run agent _dispatch-helpers
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/remote-finish.log" ]
  grep -q -- '--id 1' "$STUB_HOME/remote-finish.log"
  grep -q -- '--state failed' "$STUB_HOME/remote-finish.log"
  grep -q 'LEAD_URL' "$STUB_HOME/remote-finish.log"
}

@test "dispatch_helpers fails queued remote helpers when WORKER_AGENTS is empty and LEAD_URL is unset" {
  stub_agentbus_remote
  echo 'WORKER_AGENTS=()' >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":1,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  run agent _dispatch-helpers
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/remote-finish.log" ]
  grep -q -- '--id 1' "$STUB_HOME/remote-finish.log"
  grep -q -- '--state failed' "$STUB_HOME/remote-finish.log"
  grep -q 'LEAD_URL' "$STUB_HOME/remote-finish.log"
}

@test "dispatch_helpers preserves queued remote helpers when LEAD_URL is set" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":1,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  run agent _dispatch-helpers
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]
}

@test "node_poll submits queued remote helpers to lead and retains on outage" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":2,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  # First run: lead submission fails (outage)
  touch "$STUB_HOME/submit-fail"
  run agent node --once
  [ "$status" -eq 0 ]
  [ -f "$STUB_HOME/remote-prepare.log" ]
  grep -q 'id=2' "$STUB_HOME/remote-prepare.log"
  [ -f "$STUB_HOME/last-job-submit.json" ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]

  # Second run: outage recovered, submission succeeds
  rm "$STUB_HOME/submit-fail"
  run agent node --once
  [ "$status" -eq 0 ]
  grep -q '"event":"node_helper_submitted"' "$AGENTS_HOME/logs/agent.jsonl"
  [ -f "$STUB_HOME/helper-set.log" ]
  grep -q -- '--id 2' "$STUB_HOME/helper-set.log"
  grep -q -- '--status running' "$STUB_HOME/helper-set.log"
}

@test "node_poll polls terminal helper results and delivers ordinary helper result" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":3,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"running","remote_job":{"ID":"node-test-h3","Repository":"repo","BaseRef":"main","BaseSHA":"0123456789abcdef0123456789abcdef01234567","Requires":["hw:rp2350"],"Selector":"worker","Task":"test task","Requester":"member@node-test"}}]
EOF

  cat > "$STUB_HOME/job-get.json" <<'EOF'
{"Request":{"ID":"node-test-h3","Repository":"repo","BaseRef":"main","BaseSHA":"0123456789abcdef0123456789abcdef01234567","Requires":["hw:rp2350"],"Selector":"worker","Task":"test task","Requester":"member@node-test"},"State":"done","Node":"node-remote","Branch":"agents/worker/h3","Result":"tests pass 100%","PRURL":"https://github.com/example/pr/1"}
EOF

  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/last-job-submit.json" ]
  [ -f "$STUB_HOME/remote-finish.log" ]
  grep -q -- '--id 3' "$STUB_HOME/remote-finish.log"
  grep -q -- '--state done' "$STUB_HOME/remote-finish.log"
  grep -q -- '--node node-remote' "$STUB_HOME/remote-finish.log"
  grep -q -- '--branch agents/worker/h3' "$STUB_HOME/remote-finish.log"
  grep -q -- '--result tests pass 100%' "$STUB_HOME/remote-finish.log"
  grep -q -- '--pr https://github.com/example/pr/1' "$STUB_HOME/remote-finish.log"
}

@test "node_poll rejects mismatched job response and retains remote helper" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":4,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"running","remote_job":{"ID":"node-test-h4","Repository":"repo","BaseRef":"main","BaseSHA":"0123456789abcdef0123456789abcdef01234567","Requires":["hw:rp2350"],"Selector":"worker","Task":"test task","Requester":"member@node-test"}}]
EOF

  cat > "$STUB_HOME/job-get.json" <<'EOF'
{"Request":{"ID":"node-test-h4","Repository":"repo","BaseRef":"main","BaseSHA":"ffffffffffffffffffffffffffffffffffffffff","Requires":["hw:rp2350"],"Selector":"worker","Task":"test task","Requester":"member@node-test"},"State":"done","Node":"node-remote","Branch":"agents/worker/h4","Result":"done"}
EOF

  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]
  grep -q '"event":"node_helper_reply_invalid"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "node_poll rejects mismatched POST job response and retains queued helper" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":8,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  touch "$STUB_HOME/post-mismatch"

  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/helper-set.log" ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]
  grep -q '"event":"node_helper_reply_invalid"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "node_poll fails remote helper when helper-remote-prepare reports unpushed commit" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":5,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  echo "helper_remote_prepare: commit not pushed to origin; push your branch first" > "$STUB_HOME/prepare-fail"
  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/last-job-submit.json" ]
  [ -f "$STUB_HOME/remote-finish.log" ]
  grep -q -- '--id 5' "$STUB_HOME/remote-finish.log"
  grep -q -- '--state failed' "$STUB_HOME/remote-finish.log"
  grep -q -- 'not pushed' "$STUB_HOME/remote-finish.log"
  grep -q -- 'push' "$STUB_HOME/remote-finish.log"
}

@test "node_poll retries remote helper when helper-remote-prepare fails with storage error" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":6,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"queued"}]
EOF

  touch "$STUB_HOME/prepare-storage-err"
  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]
  grep -q '"event":"node_helper_prepare_storage_failed"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "node_poll logs error on GET transport failure and retains remote helper" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  echo "declare -gA NODE_REPOS=([repo]=\"$HOME/repo\")" >> "$AGENTS_HOME/agents.conf"
  cat > "$STUB_HOME/remote-helpers.json" <<'EOF'
[{"id":7,"from":"member","from_task":"task-1","requires":["hw:rp2350"],"status":"running","remote_job":{"ID":"node-test-h7","Repository":"repo","BaseRef":"main","BaseSHA":"0123456789abcdef0123456789abcdef01234567","Requires":["hw:rp2350"],"Selector":"worker","Task":"test task","Requester":"member@node-test"}}]
EOF

  touch "$STUB_HOME/get-transport-fail"
  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]
  grep -q '"event":"node_helper_get_failed"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "maximum-size escaped helper task delivers its terminal GET result" {
  stub_agentbus_remote
  echo 'LEAD_URL=http://127.0.0.1:7780' >> "$AGENTS_HOME/agents.conf"
  echo 'INSTANCE_NAME=node-test' >> "$AGENTS_HOME/agents.conf"
  # A running helper bypasses POST, exercising GET validation independently.
  python3 - "$STUB_HOME" <<'MAKE'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
r = {"ID":"large-task", "Repository":"repo", "BaseRef":"main",
     "BaseSHA":"a"*40, "Requires":["hw:rp2350"], "Selector":"worker",
     "Target":"", "Task":'"'*65536, "Requester":"member@node-test"}
(p/"remote-helpers.json").write_text(json.dumps([
    {"id":1,"status":"running","remote_job":r}]))
(p/"job-get.json").write_text(json.dumps({"Request":r,"State":"done",
    "Node":"node2","Branch":"result","Result":"completed"}))
MAKE
  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/last-job-submit.json" ]
  [ -f "$STUB_HOME/remote-finish.log" ]
  grep -q -- '--id 1 --state done --node node2 --branch result --result completed' "$STUB_HOME/remote-finish.log"
}
