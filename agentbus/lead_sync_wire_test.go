package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestLeadSyncCountsWireBytes accepts valid JSON without inflating HTML text during validation.
func TestLeadSyncCountsWireBytes(t *testing.T) {
	a, b := syncFixture(t)
	b.Files = []leadSyncFile{syncFile("prompt-1.md", strings.Repeat("<", 256*1024)), syncFile("prompt-2.md", strings.Repeat("<", 256*1024))}
	var data bytes.Buffer // jq sends literal HTML characters, which need no JSON escaping
	enc := json.NewEncoder(&data)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(b); err != nil {
		t.Fatal(err)
	}
	if data.Len() > 2*1024*1024 {
		t.Fatal("fixture exceeds wire bound")
	}
	r := httptest.NewRequest(http.MethodPost, "/v1/sync", &data)
	r.RemoteAddr = "127.0.0.1:1"
	w := httptest.NewRecorder()
	newLeadHandler(a.bus, "node-a", nil, a.ttl).ServeHTTP(w, r)
	requireLeadStatus(t, w, http.StatusOK)
}

// TestLeadSyncCleanupCannotBreakHeartbeat preserves registry service when mirror cleanup needs retrying.
func TestLeadSyncCleanupCannotBreakHeartbeat(t *testing.T) {
	a, b := syncFixture(t)
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(a.bus.dir, "remote")
	if _, err := os.Stat(filepath.Join(root, "node-a", "task-a", "final.md")); err != nil {
		t.Fatal(err)
	}
	if err := os.RemoveAll(root); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(root, []byte("disk unavailable"), 0600); err != nil {
		t.Fatal(err)
	}
	_, err := a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil)})
	if err != nil {
		t.Fatalf("mirror cleanup broke heartbeat: %v", err)
	}
}

// TestLeadSyncNodeLogScope accepts another owned job's records while excluding other nodes' task logs.
func TestLeadSyncNodeLogScope(t *testing.T) {
	a, b := syncFixture(t)
	for _, node := range []string{"node-a", "node-b"} {
		req := regRequest("job-" + node)
		req.Target = node
		if _, err := a.submit(node, true, req); err != nil {
			t.Fatal(err)
		}
		job, err := a.heartbeat(node, leadHeartbeatBody{NodeHeartbeat: regBeat(node, nil, FreeAgent{Name: "alpha", Tier: TierNormal})})
		if err != nil || job == nil {
			t.Fatalf("assign: %v", err)
		}
		if _, err := a.report(node, job.Request.ID, JobReport{Assignment: job.Assignment, State: JobRunning, TaskID: "task-" + node}); err != nil {
			t.Fatal(err)
		}
	}
	b.Logs = []json.RawMessage{json.RawMessage(`{"level":"info","task":"task-node-a","event":"other_owned_job"}`)}
	if err := a.sync("node-a", b); err != nil {
		t.Fatalf("owned task log refused: %v", err)
	}
	data, err := os.ReadFile(filepath.Join(a.bus.dir, "remote", "node-a", "logs.jsonl"))
	if err != nil || !bytes.Contains(data, []byte("other_owned_job")) {
		t.Fatalf("owned log not stored: %q %v", data, err)
	}
	b.Logs = []json.RawMessage{json.RawMessage(`{"level":"info","task":"task-node-b"}`)}
	failsWith(t, LeadSyncInvalidLogs)("other node's info log", a.sync("node-a", b))
}
