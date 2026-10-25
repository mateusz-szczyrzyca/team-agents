#!/usr/bin/env bats
# The lead's listener as the `agent lead` adapter starts it (agentlead.service runs this). The adapter only passes
# LEAD_LISTEN, LEAD_NODES and INSTANCE_NAME on to `agentbus lead`, which decides; no network is used here.

load helpers

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
