#!/usr/bin/env bats
# The lead's listener as the `agent lead` adapter starts it (agentlead.service runs this). The adapter only passes
# LEAD_LISTEN, LEAD_NODES and INSTANCE_NAME on to `agentbus lead`, which decides; no network is used here.

load helpers

# teardown — cleans up the foreground lead even when an assertion fails; this file starts no transient units.
teardown() {
  [[ -f "$HOME/lead.pid" ]] || return 0
  local lead_pid rc=0
  lead_pid=$(cat "$HOME/lead.pid")
  # A process already reaped by the test is an expected non-running answer.
  if kill -0 "$lead_pid" 2> "$HOME/lead-cleanup.log"; then
    if ! kill -TERM "$lead_pid" 2>> "$HOME/lead-cleanup.log"; then
      cat "$HOME/lead-cleanup.log" >&2
      return 1
    fi
    wait "$lead_pid" || rc=$?
    if [[ $rc -ne 0 && $rc -ne 143 ]]; then
      cat "$HOME/lead.log" >&2
      return 1
    fi
  fi
}

@test "agent lead needs LEAD_LISTEN and says how to set it" {
  run agent lead
  [ "$status" -ne 0 ]
  [[ $output == *"LEAD_LISTEN is not set"* ]]
}

@test "agent lead refuses a public listen address (agentbus lead decides)" {
  run env LEAD_LISTEN=8.8.8.8:7780 agent lead
  [ "$status" -ne 0 ]
  [[ $output == *"neither loopback nor a Tailscale address"* ]]
}

@test "agent lead serves its API on its address and on 127.0.0.1, and stops on SIGTERM" {
  local port
  port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.2", 0)); print(s.getsockname()[1])')
  echo "LEAD_LISTEN=127.0.0.2:$port" >> "$AGENTS_HOME/agents.conf"
  agent lead > "$HOME/lead.log" 2>&1 &
  local pid=$!
  echo "$pid" > "$HOME/lead.pid"
  wait_grep '"event":"lead_listening"' "$AGENTS_HOME/logs/agentbus.jsonl" 15
  run curl -fsS --noproxy '*' --max-time 2 "http://127.0.0.2:$port/v1/nodes"
  [ "$status" -eq 0 ]
  run curl -fsS --noproxy '*' --max-time 2 "http://127.0.0.1:$port/v1/nodes"
  [ "$status" -eq 0 ]
  kill -TERM "$pid"
  local rc=0
  wait "$pid" || rc=$?
  [ "$rc" -eq 0 ]
  rm "$HOME/lead.pid"
  run curl -fsS --noproxy '*' --max-time 2 "http://127.0.0.2:$port/v1/nodes"
  [ "$status" -ne 0 ]
}
