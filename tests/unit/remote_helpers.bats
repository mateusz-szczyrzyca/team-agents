#!/usr/bin/env bats
# Tests for node daemon remote delegation hooks (dispatch_helpers and node_poll helper handling).

load helpers

teardown() {
  for p in "$STUB_UNITS"/*.pid; do
    [[ -f $p ]] && kill -TERM -- -"$(cat "$p")" 2>/dev/null || true
  done
}

# stub_agentbus_remote intercepts helper-remote-* and node-request commands
stub_agentbus_remote() {
  local real_bus="$HOME/.local/bin/agentbus"
  if [[ ! -f "$HOME/.local/bin/real-agentbus" ]]; then
    mv "$real_bus" "$HOME/.local/bin/real-agentbus"
  fi
  cat > "$real_bus" <<'STUB'
#!/usr/bin/env bash
cmd=${1:-}
case "$cmd" in
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
    if [[ -f "$STUB_HOME/prepare-fail" ]]; then
      exit 1
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
      printf '{"State":"queued"}\n'
      exit 0
    fi
    if [[ $path == /v1/jobs/* ]]; then
      if ((is_get)); then
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
  # Replace real-agentbus helper-next with logger
  local bus_bin="$HOME/.local/bin/real-agentbus"
  cat > "$bus_bin" <<'STUB'
#!/usr/bin/env bash
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
{"Request":{"ID":"node-test-h3"},"State":"done","Node":"node-remote","Branch":"agents/worker/h3","Result":"tests pass 100%","PRURL":"https://github.com/example/pr/1"}
EOF

  run agent node --once
  [ "$status" -eq 0 ]
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

  # Lead returns response for another job ID
  cat > "$STUB_HOME/job-get.json" <<'EOF'
{"Request":{"ID":"different-job-id"},"State":"done","Node":"node-remote","Branch":"agents/worker/h4","Result":"done"}
EOF

  run agent node --once
  [ "$status" -eq 0 ]
  [ ! -f "$STUB_HOME/remote-finish.log" ]
  grep -q '"event":"node_helper_reply_invalid"' "$AGENTS_HOME/logs/agent.jsonl"
}
