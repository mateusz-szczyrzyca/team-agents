package main

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestRemoteAgentDetailNonterminalJobOrder verifies that nonterminal jobs (such as waiting or
// assigned jobs) appear before finished/completed jobs on the remote agent detail view (/agents/<id>).
func TestRemoteAgentDetailNonterminalJobOrder(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	snap, err := readWebJobs(filepath.Join(home, "bus"))
	if err != nil {
		t.Fatal(err)
	}
	// Set the active job for worker-1 to "waiting"
	snap.Jobs[0].State = "waiting"
	data, e := json.Marshal(snap)
	if e != nil {
		t.Fatal(e)
	}
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), string(data))

	_, ts := testServer(t, home)
	_, body := get(t, ts, "/agents/worker-1@node-online")
	if !before(body, `href="/jobs/job-active"`, `href="/jobs/job-recent-done"`) {
		t.Fatal("current waiting job appears below completed jobs on remote agent detail page")
	}
}

// TestRemoteAgentOfflineAvailability verifies that when a node is offline (last seen exceeds TTL),
// remote agent list and detail views do not show stale "free" status, but display "offline" availability.
func TestRemoteAgentOfflineAvailability(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	snap, err := readWebJobs(filepath.Join(home, "bus"))
	if err != nil {
		t.Fatal(err)
	}
	node := snap.Nodes["node-online"]
	node.LastSeen = time.Now().Add(-48 * time.Hour)
	snap.Nodes["node-online"] = node
	data, e := json.Marshal(snap)
	if e != nil {
		t.Fatal(e)
	}
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), string(data))

	stubLocalAgents(t)
	_, ts := testServer(t, home)

	// Detail view /agents/agent-free@node-online
	_, bodyDetail := get(t, ts, "/agents/agent-free@node-online")
	if strings.Contains(bodyDetail, `class="st st-dim">free</span>`) {
		t.Fatal("node offline for 48 hours, agent detail view still shown as free")
	}
	mustContain(t, "/agents/agent-free@node-online", bodyDetail, `class="st st-offline">offline</span>`)

	// List view /agents
	_, bodyList := get(t, ts, "/agents")
	mustContain(t, "/agents", bodyList, `class="st st-offline">offline</span>`)
}
