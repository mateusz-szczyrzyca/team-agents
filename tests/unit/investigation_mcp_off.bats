#!/usr/bin/env bats
# An independent investigator (an investigation team's first phase) has no bus: its CLI is told of the agentbus server
# switched off. Codex and opencode want every server they are told of to say how to reach it — "invalid transport in
# mcp_servers.agentbus" stopped inv-astra at its start: the server is given its command, and switched off.

load helpers

@test "codex: an independent investigator's agentbus server has its command and is off; anyone else's is on" {
  out=$(source "$REPO_ROOT/bin/agent"
    is_independent_investigation() { return 0; }
    bus_args_json() { echo '["serve"]'; }
    codex_mcp_args X)
  [[ $out == *'mcp_servers.agentbus.command='* && $out == *'mcp_servers.agentbus.enabled=false'* ]]
  out=$(source "$REPO_ROOT/bin/agent"
    is_independent_investigation() { return 1; }
    bus_args_json() { echo '["serve"]'; }
    codex_mcp_args X)
  [[ $out == *'mcp_servers.agentbus.command='* && $out == *'mcp_servers.agentbus.args=["serve"]'* ]]
  [[ $out != *'enabled=false'* ]]
}

@test "opencode: an independent investigator's agentbus server says how to reach it, and is off" {
  cfg=$(source "$REPO_ROOT/bin/agent"
    is_independent_investigation() { return 0; }
    bus_args_json() { echo '["serve"]'; }
    opencode_config X)
  [ "$(jq -r '.mcp.agentbus.enabled' <<< "$cfg")" = false ]
  [ "$(jq -r '.mcp.agentbus.type' <<< "$cfg")" = local ]
  [ "$(jq -r '.mcp.agentbus.command | length' <<< "$cfg")" -ge 1 ]
}
