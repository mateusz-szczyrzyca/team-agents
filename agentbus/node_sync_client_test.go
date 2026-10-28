package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestNodeSyncClientEnvelope permits large syncs without broadening ordinary request limits.
func TestNodeSyncClientEnvelope(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/sync" {
			t.Errorf("route: %s", r.URL.Path)
		}
		writeLeadJSON(w, 200, struct{ OK bool }{true})
	}))
	defer srv.Close()
	payload := `{"Content":"` + strings.Repeat("x", 300*1024) + `"}`
	cfg := nodeRequestConfig{url: srv.URL, path: "/v1/sync"}
	if _, err := nodeRequest(cfg, strings.NewReader(payload)); err != nil {
		t.Fatalf("valid sync envelope refused: %v", err)
	}
	cfg.path = "/v1/heartbeat"
	failsWith(t, NodeClientInvalidArgument)("ordinary envelope unchanged", func() *NodeClientError { _, err := nodeRequest(cfg, strings.NewReader(payload)); return err }())
	cfg.path = "/v1/sync"
	failsWith(t, NodeClientInvalidArgument)("oversized sync", func() *NodeClientError {
		_, err := nodeRequest(cfg, strings.NewReader(strings.Repeat(" ", 2*1024*1024+1)))
		return err
	}())
}

// TestLeadSyncLogViewRecovery repairs failed materialization without losing or duplicating an accepted log batch.
func TestLeadSyncLogViewRecovery(t *testing.T) {
	a, b := syncFixture(t)
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(a.bus.dir, "remote", "node-a", "logs.jsonl")
	if err := os.Mkdir(path, 0700); err != nil {
		t.Fatal(err)
	}
	b.Logs = []json.RawMessage{json.RawMessage(`{"level":"warn","event":"once"}`)}
	failsWith(t, LeadSyncStorage)("failed log view", a.sync("node-a", b))
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	a = &leadAPI{bus: &Bus{dir: a.bus.dir}, ttl: time.Minute}
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path) // #nosec G304 -- path is inside newBus(t), using fixed test identifiers.
	if err != nil || strings.Count(string(data), "\n") != 1 {
		t.Fatalf("log recovery: %q %v", data, err)
	}
}
