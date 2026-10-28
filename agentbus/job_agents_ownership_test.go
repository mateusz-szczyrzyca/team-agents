package main

import "testing"

// TestHeartbeatConfiguredAgentsOwnsInput keeps the registry independent of caller mutation.
func TestHeartbeatConfiguredAgentsOwnsInput(t *testing.T) {
	r := newLeadRegistry()
	agents := []NodeAgent{{Name: "busy", Type: "codex", Model: "original", Tier: TierNormal}}
	regHeartbeat(t, r, NodeHeartbeat{Node: "node-a", Agents: agents}, regAt(0))
	agents[0].Model = "mutated"
	got := r.nodes["node-a"].Heartbeat.Agents
	if len(got) != 1 || got[0].Model != "original" {
		t.Fatalf("registry does not own agent declaration: %+v", got)
	}
}
