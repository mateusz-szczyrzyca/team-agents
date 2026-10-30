package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestLeadAnswerStateIntegrity verifies LeadAnswerInvalidState and LeadAnswerStorage rejection causes.
func TestLeadAnswerStateIntegrity(t *testing.T) {
	// Job running in accepted registry must fail with LeadAnswerInvalidState
	aRun, _ := setupRunningJob(t)
	content := "Ans\n**ANSWERED**"
	_, errNotWaiting := aRun.answer("node-a", "t1", "questions-1.md", content)
	failsWith(t, LeadAnswerInvalidState)("running job answer", errNotWaiting)

	// Corrupt jobs.json must fail with LeadAnswerStorage
	aStorage, _ := setupWaitingJob(t)
	if err := os.WriteFile(filepath.Join(aStorage.bus.dir, "jobs.json"), []byte("{corrupt"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, errStorage := aStorage.answer("node-a", "t1", "questions-1.md", content)
	failsWith(t, LeadAnswerStorage)("corrupt jobs.json", errStorage)
}

// TestLeadHeartbeatResponseWireBudgetWithWakes verifies that wakes are counted in wire budget calculations.
func TestLeadHeartbeatResponseWireBudgetWithWakes(t *testing.T) {
	a, first := setupWaitingJob(t)
	req := JobRequest{ID: "job-wire-2", Repository: "repo", BaseRef: "main", Selector: "worker", Task: "second", Requester: "human"}
	if _, err := a.submit("lead", true, req); err != nil {
		t.Fatal(err)
	}
	second, err := a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: NodeHeartbeat{Node: "node-a", FreeAgents: []FreeAgent{{Name: "worker-2", Tier: TierWorker}}}})
	if err != nil || second == nil || second.Request.ID == first {
		t.Fatalf("distinct assignment: %v %v", second, err)
	}
	now := time.Now().UTC()
	for i, state := range []JobState{JobRunning, JobWaiting} {
		if _, err := a.report("node-a", second.Request.ID, JobReport{ID: second.Request.ID, Node: "node-a", Assignment: second.Assignment, State: state, TaskID: "t2", At: Timestamp{now.Add(time.Duration(i) * time.Second)}}); err != nil {
			t.Fatal(err)
		}
	}

	// Two valid answers near 256 KiB with newlines (~523.6 KiB encoded each; together ~1,047,200 bytes)
	size := 261800
	content := strings.Repeat("\n", size-len("\n**ANSWERED**")) + "\n**ANSWERED**"
	answers := []LeadAnswer{
		{Job: first, TaskID: "t1", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)},
		{Job: second.Request.ID, TaskID: "t2", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)},
	}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{now}, func(r *leadRegistry) *LeadHeartbeatError {
		for _, ans := range answers {
			job := r.jobs[ans.Job]
			job.PendingAnswers = []LeadAnswer{ans}
			r.jobs[ans.Job] = job
		}
		// Populate wakes: 200 remote members whose wake is true (more than 2,800 encoded bytes of wakes)
		for i := 1; i <= 200; i++ {
			name := fmt.Sprintf("remote-worker-%02d@node-a", i)
			r.members[name] = RemoteMember{
				Member: name,
				Node:   "node-a",
				Job:    fmt.Sprintf("wake-job-%02d", i),
				Wake:   true,
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	hb := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node: "node-a",
		},
	}
	payload, jerr := json.Marshal(hb)
	if jerr != nil {
		t.Fatal(jerr)
	}
	reqHTTP := httptest.NewRequest(http.MethodPost, "/v1/heartbeat", strings.NewReader(string(payload)))
	reqHTTP.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	newLeadHandler(a.bus, "node-a", nil, a.ttl).ServeHTTP(w, reqHTTP)
	if w.Code != http.StatusOK {
		t.Fatalf("heartbeat status: %d", w.Code)
	}
	if w.Body.Len() > 1<<20 {
		t.Fatalf("reply exceeds client cap: %d", w.Body.Len())
	}
	var reply struct {
		Answers []LeadAnswer // controls delivered in this response
		Wakes   []string     // wake requests preserved by control bounding
	}
	if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
		t.Fatal(err)
	}
	if len(reply.Wakes) != 200 {
		t.Fatalf("expected 200 wakes, got %d", len(reply.Wakes))
	}
	// Under 1 MiB wire limit with 200 wakes, only 1 of the two ~500 KiB answers fits
	if len(reply.Answers) != 1 {
		t.Fatalf("expected 1 answer due to wakes pushing 2nd answer over 1 MiB, got %d answers", len(reply.Answers))
	}
}
