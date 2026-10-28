package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// TestLeadSyncCommittedLogRetry exercises failure after the canonical log batch commits, then repairs its view.
func TestLeadSyncCommittedLogRetry(t *testing.T) {
	a, b := syncFixture(t)
	b.Logs = []json.RawMessage{json.RawMessage(`{"level":"warn","event":"first"}`)}
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(a.bus.dir, "remote", "node-a", "logs.jsonl")
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(path, 0700); err != nil {
		t.Fatal(err)
	}
	b.Logs = []json.RawMessage{json.RawMessage(`{"level":"warn","event":"committed"}`)}
	failsWith(t, LeadSyncStorage)("view replacement blocked", a.sync("node-a", b))
	state, err := os.ReadFile(filepath.Join(a.bus.dir, "remote", "node-a", "logs-state.json"))
	if err != nil || !bytes.Contains(state, []byte("committed")) {
		t.Fatalf("batch did not commit before view failure: %q %v", state, err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path) // #nosec G304 -- path is inside newBus(t) with fixed test identifiers.
	if err != nil || bytes.Count(data, []byte("\n")) != 2 || bytes.Count(data, []byte("committed")) != 1 {
		t.Fatalf("committed retry duplicated or lost logs: %q %v", data, err)
	}
}
