#!/usr/bin/env bats
# The node daemon (`agent node`). It connects to the lead over HTTP, sends periodic heartbeats,
# takes jobs matching its declared capabilities and free agents, starts tasks through the shared
# runner path, and recovers durably across crashes and restarts.

load helpers

teardown() {
  if [[ -f "$HOME/node.pid" ]]; then
    local node_pid
    node_pid=$(cat "$HOME/node.pid")
    if kill -0 "$node_pid" 2>/dev/null; then
      kill -TERM "$node_pid" 2>/dev/null || true
      wait "$node_pid" 2>/dev/null || true
    fi
    rm -f "$HOME/node.pid"
  fi
}

@test "agent node requires LEAD_URL and says how to set it" {
  run agent node
  [ "$status" -ne 0 ]
  [[ $output == *"LEAD_URL is not set: set it in config/agents.conf (e.g. LEAD_URL=http://lead-instance:7780)"* ]]
}
