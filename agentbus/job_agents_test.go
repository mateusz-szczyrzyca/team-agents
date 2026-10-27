package main

// Tests of the configured agents a node declares in its heartbeat (decision 4, written first, docs/tdd.md):
// the lead validates and stores Agents [{Name, Type, Model, Tier}] alongside FreeAgents, keeps routing on
// FreeAgents only, and loads jobs.json snapshots written before Agents existed.

import (
	"fmt"
	"reflect"
	"strings"
	"testing"
	"time"
)

// Rule: a heartbeat stores all configured agents of the node (including busy ones) in the registry and in
// jobs.json, returns an owned copy, and routes queued jobs by FreeAgents only.
func TestHeartbeatStoresConfiguredAgentsAndRoutesByFreeAgents(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Minute}
	if _, err := a.submit("node-a", true, regRequest("job-normal")); err != nil {
		t.Fatalf("submit: %v", err)
	}
	configured := []NodeAgent{
		{Name: "alpha", Type: "claude", Model: "sonnet", Tier: TierNormal},
		{Name: "worker-1", Type: "opencode", Model: "openrouter/@preset/qwen-worker", Tier: TierWorker},
	}
	beat := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node:         "node-a",
			Capabilities: []string{"os:linux", "arch:arm64"},
			Agents:       configured,
			FreeAgents:   []FreeAgent{{Name: "worker-1", Tier: TierWorker}},
		},
	}
	assigned, err := a.heartbeat("node-a", beat)
	if err != nil {
		t.Fatalf("heartbeat: %v", err)
	}
	if assigned != nil {
		t.Fatalf("busy normal agent in Agents was used for routing: got %+v, want nil", assigned)
	}
	// Mutating caller slice after heartbeat must not alter stored declaration.
	configured[0].Model = "mutated"

	nodes, lerr := a.listNodes()
	if lerr != nil {
		t.Fatalf("listNodes: %v", lerr)
	}
	want := []NodeAgent{
		{Name: "alpha", Type: "claude", Model: "sonnet", Tier: TierNormal},
		{Name: "worker-1", Type: "opencode", Model: "openrouter/@preset/qwen-worker", Tier: TierWorker},
	}
	if len(nodes) != 1 || !reflect.DeepEqual(nodes[0].Agents, want) {
		t.Fatalf("stored Agents = %+v, want %+v", nodes, want)
	}
	jobView, gerr := a.get("job-normal")
	if gerr != nil {
		t.Fatalf("get: %v", gerr)
	}
	if jobView.WaitReason != "no free normal agent" {
		t.Fatalf("WaitReason = %q, want \"no free normal agent\"", jobView.WaitReason)
	}
}

// Rule: invalid configured agent entries (bad or duplicate Name, bad Type, unknown Tier, control characters
// or oversized Model, or exceeding nodeAgentsMax) are refused with LeadHeartbeatMalformedBody and leave the
// node's previous snapshot intact.
func TestHeartbeatRejectsInvalidConfiguredAgents(t *testing.T) {
	manyAgents := make([]NodeAgent, nodeAgentsMax+1)
	for i := range manyAgents {
		manyAgents[i] = NodeAgent{Name: fmt.Sprintf("a-%d", i), Type: "claude", Tier: TierNormal}
	}
	cases := []struct {
		name   string
		agents []NodeAgent
	}{
		{"empty agent name", []NodeAgent{{Name: "", Type: "claude", Tier: TierNormal}}},
		{"path in agent name", []NodeAgent{{Name: "../alpha", Type: "claude", Tier: TierNormal}}},
		{"duplicate agent name", []NodeAgent{
			{Name: "alpha", Type: "claude", Tier: TierNormal},
			{Name: "alpha", Type: "codex", Tier: TierWorker},
		}},
		{"empty agent type", []NodeAgent{{Name: "alpha", Type: "", Tier: TierNormal}}},
		{"invalid agent type", []NodeAgent{{Name: "alpha", Type: "claude/code", Tier: TierNormal}}},
		{"unknown agent tier", []NodeAgent{{Name: "alpha", Type: "claude", Tier: Tier("investigator")}}},
		{"newline in model", []NodeAgent{{Name: "alpha", Type: "claude", Model: "sonnet\nx", Tier: TierNormal}}},
		{"nul in model", []NodeAgent{{Name: "alpha", Type: "claude", Model: "sonnet\x00", Tier: TierNormal}}},
		{"model exceeds max length", []NodeAgent{{Name: "alpha", Type: "claude", Model: strings.Repeat("m", nodeAgentModelMax+1), Tier: TierNormal}}},
		{"too many configured agents", manyAgents},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			r := newLeadRegistry()
			initial := NodeHeartbeat{
				Node:       "node-a",
				Agents:     []NodeAgent{{Name: "alpha", Type: "claude", Model: strings.Repeat("m", nodeAgentModelMax), Tier: TierNormal}},
				FreeAgents: []FreeAgent{{Name: "alpha", Tier: TierNormal}},
			}
			regHeartbeat(t, r, initial, regAt(1))
			_, err := r.heartbeat(NodeHeartbeat{Node: "node-a", Agents: tc.agents}, regAt(2))
			failsWith(t, LeadHeartbeatMalformedBody)(tc.name, err)
			if got := r.nodes["node-a"]; !got.LastSeen.Equal(regAt(1).Time) || len(got.Heartbeat.Agents) != 1 {
				t.Fatalf("refused heartbeat altered stored node: %+v", got)
			}
		})
	}
}

// Rule: a jobs.json written before NodeHeartbeat.Agents existed still loads with its node declaration intact,
// and existing FreeAgents validation remains unchanged for compatibility (AGENTS.md rule 9).
func TestJobStoreLoadsNodeBeforeAgentsField(t *testing.T) {
	r, err := (&Bus{dir: "testdata/jobs-before-node-agents"}).loadJobs()
	if err != nil {
		t.Fatalf("loadJobs: %v", err)
	}
	node, ok := r.nodes["node-a"]
	if !ok {
		t.Fatal("stored node-a missing after load")
	}
	if len(node.Heartbeat.Agents) != 0 || len(node.Heartbeat.FreeAgents) != 1 || node.Heartbeat.FreeAgents[0].Name != "alpha" {
		t.Fatalf("loaded node declaration mismatch: %+v", node.Heartbeat)
	}
	manyFree := make([]FreeAgent, nodeAgentsMax+1)
	for i := range manyFree {
		manyFree[i] = FreeAgent{Name: fmt.Sprintf("f-%d", i), Tier: TierNormal}
	}
	if _, herr := r.heartbeat(NodeHeartbeat{Node: "node-a", FreeAgents: manyFree}, regAt(2)); herr != nil {
		t.Fatalf("legacy FreeAgents list rejected: %v", herr)
	}
}
