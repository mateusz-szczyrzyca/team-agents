package main

import (
	"bytes"
	"encoding/json"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// assertNodeTransportRecovery checks the transport/registry half of two-installation recovery.
// node_two_homes.bats separately drives the real daemon's durable outbox and stub runner.
func assertNodeTransportRecovery(t *testing.T) {
	t.Helper()
	lead := newBus(t)
	nodeHome := t.TempDir()
	a := &leadAPI{bus: lead, ttl: time.Minute}
	req := regRequest("outage")
	if _, err := a.submit("node-a", true, req); err != nil {
		t.Fatal(err)
	}
	job, err := a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})})
	if err != nil || job == nil {
		t.Fatalf("assignment=%+v, %v", job, err)
	}
	if _, err := a.report("node-a", req.ID, JobReport{Assignment: job.Assignment, State: JobRunning, TaskID: "local-task", Branch: "agents/alpha/result"}); err != nil {
		t.Fatal(err)
	}
	current, getErr := a.get(req.ID)
	if getErr != nil {
		t.Fatal(getErr)
	}
	at := Timestamp{current.History[len(current.History)-1].At.Add(time.Second)}
	report := JobReport{ID: req.ID, Assignment: job.Assignment, State: JobDone, TaskID: "local-task", Branch: "agents/alpha/result", Result: "completed while disconnected", At: at}
	body, marshalErr := json.Marshal(leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil), Reports: []JobReport{report}})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	pending := filepath.Join(nodeHome, "pending.json")
	if err := os.WriteFile(pending, body, 0o600); err != nil {
		t.Fatal(err)
	}
	server := httptest.NewServer(newLeadHandler(lead, "node-a", nil, time.Minute))
	cfg, clientErr := parseNodeRequest([]string{"--url", server.URL, "--path", "/v1/heartbeat"})
	if clientErr != nil {
		t.Fatal(clientErr)
	}
	server.Close()
	_, clientErr = nodeRequest(cfg, bytes.NewReader(body))
	failsWith(t, NodeClientUnreachable)("lead outage", clientErr)
	saved, readErr := os.ReadFile(pending) // #nosec G304 -- the snapshot is confined to t.TempDir, never a job-supplied path
	if readErr != nil || !bytes.Equal(saved, body) {
		t.Fatal("pending snapshot changed during outage")
	}
	server = httptest.NewServer(newLeadHandler(lead, "node-a", nil, time.Minute))
	defer server.Close()
	cfg.url = server.URL
	for retry := 0; retry < 2; retry++ {
		if _, err := nodeRequest(cfg, bytes.NewReader(saved)); err != nil {
			t.Fatal(err)
		}
	}
	final, getErr := (&leadAPI{bus: lead, ttl: time.Minute}).get(req.ID)
	if getErr != nil || final.State != JobDone || final.Result != report.Result {
		t.Fatalf("final=%+v, %v", final, getErr)
	}
	if len(final.History) != 4 || !final.History[3].At.Equal(at.Time) {
		t.Fatalf("duplicate transition or lost time: %+v", final.History)
	}
}
