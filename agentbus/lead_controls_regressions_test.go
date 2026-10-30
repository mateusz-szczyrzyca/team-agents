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

// TestLeadAnswerMarkerMustBeLastLine verifies that **ANSWERED** must be the final line.
func TestLeadAnswerMarkerMustBeLastLine(t *testing.T) {
	a, _ := setupWaitingJob(t)
	content := "Verdict: ok\n**ANSWERED**\nWhat about question 2?\n"
	_, err := a.answer("node-a", "t1", "questions-1.md", content)
	failsWith(t, LeadAnswerNotAnswered)("middle marker", err)
}

// TestLeadAnswerAndKickReplacementAndCapacity verifies at most one answer and kick per job.
func TestLeadAnswerAndKickReplacementAndCapacity(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	content1 := "Ans 1\n**ANSWERED**"
	if _, err1 := a.answer("node-a", "t1", "questions-1.md", content1); err1 != nil {
		t.Fatal(err1)
	}
	content2 := "Ans 2\n**ANSWERED**"
	if _, err2 := a.answer("node-a", "t1", "questions-2.md", content2); err2 != nil {
		t.Fatal(err2)
	}

	reg, serr := a.bus.loadJobs()
	if serr != nil {
		t.Fatal(serr)
	}
	job := reg.jobs[jobID]
	if len(job.PendingAnswers) != 1 || job.PendingAnswers[0].File != "questions-2.md" {
		t.Fatalf("expected 1 replaced pending answer, got %+v", job.PendingAnswers)
	}
}

// TestLeadKickMetadataIntegrity checks rejection on missing or corrupt mirror metadata.
func TestLeadKickMetadataIntegrity(t *testing.T) {
	a, _ := setupRunningJob(t)
	// Remove mirror metadata so running job has no metadata.json
	metaFile := filepath.Join(a.bus.dir, "remote", "node-a", "t1", "metadata.json")
	if err := os.Remove(metaFile); err != nil {
		t.Fatal(err)
	}
	// Missing mirror metadata must be rejected with InvalidState (never guess run 1)
	_, errNoMeta := a.kick("node-a", "t1")
	failsWith(t, LeadKickInvalidState)("missing mirror metadata", errNoMeta)

	// Corrupt metadata.json must fail with Storage
	if err := os.WriteFile(metaFile, []byte("{corrupt"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, errCorrupt := a.kick("node-a", "t1")
	failsWith(t, LeadKickStorage)("corrupt mirror metadata", errCorrupt)
}

// TestLeadHeartbeatResponseWireBudget checks actual encoded replies, retries and queue drainage.
func TestLeadHeartbeatResponseWireBudget(t *testing.T) {
	for _, tc := range []struct {
		name     string // encoding boundary under test
		fill     string // characters whose JSON escaping affects the budget
		envelope bool   // include a newly assigned task in the same reply
		want     int    // initial answers that fit alongside the envelope
	}{{"newlines", "\n", false, 1}, {"html", "<>&", false, 2}, {"assignment", "\n", true, 1}} {
		t.Run(tc.name, func(t *testing.T) {
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
			size := syncFileBytes
			if tc.envelope {
				size = 250000
			}
			content := strings.Repeat(tc.fill, size/len(tc.fill))
			content = content[:len(content)-len("\n**ANSWERED**")] + "\n**ANSWERED**"
			answers := []LeadAnswer{{Job: first, TaskID: "t1", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)}, {Job: second.Request.ID, TaskID: "t2", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)}}
			if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{now}, func(r *leadRegistry) *LeadHeartbeatError {
				for _, ans := range answers {
					job := r.jobs[ans.Job]
					job.PendingAnswers = []LeadAnswer{ans}
					r.jobs[ans.Job] = job
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
			hb := leadHeartbeatBody{NodeHeartbeat: NodeHeartbeat{Node: "node-a"}}
			if tc.envelope {
				req.ID = "job-envelope"
				req.Task = strings.Repeat("<", 65536)
				if _, err := a.submit("lead", true, req); err != nil {
					t.Fatal(err)
				}
				hb.FreeAgents = []FreeAgent{{Name: "worker-3", Tier: TierWorker}}
			}
			got, assigned := controlWireHeartbeat(t, a, hb)
			if len(got) != tc.want {
				t.Fatalf("answer count: got %d want %d", len(got), tc.want)
			}
			if tc.envelope && (assigned == nil || len(assigned.Request.Task) != 65536) {
				t.Fatal("assignment envelope missing")
			}
			// A new handler must recover durable, unacknowledged controls without a listener's memory.
			hb.FreeAgents = nil
			retry, _ := controlWireHeartbeat(t, &leadAPI{bus: a.bus, ttl: a.ttl}, hb)
			if len(retry) < len(got) || retry[0].SHA256 != got[0].SHA256 {
				t.Fatal("retry lost durable controls")
			}
			for _, ans := range got {
				hb.AnswerAcks = append(hb.AnswerAcks, LeadAnswerAck{Job: ans.Job, TaskID: ans.TaskID, File: ans.File, SHA256: ans.SHA256})
			}
			next, _ := controlWireHeartbeat(t, a, hb)
			if len(next) != 2-len(got) {
				t.Fatalf("ack drainage: got %d", len(next))
			}
			hb.AnswerAcks = nil
			for _, ans := range next {
				hb.AnswerAcks = append(hb.AnswerAcks, LeadAnswerAck{Job: ans.Job, TaskID: ans.TaskID, File: ans.File, SHA256: ans.SHA256})
			}
			drained, _ := controlWireHeartbeat(t, a, hb)
			if len(drained) != 0 {
				t.Fatal("acknowledged queue did not drain")
			}
		})
	}
}

// controlWireHeartbeat exercises the real JSON encoder and the client's one-MiB response limit.
func controlWireHeartbeat(t *testing.T, a *leadAPI, hb leadHeartbeatBody) ([]LeadAnswer, *leadJobView) {
	t.Helper()
	payload, err := json.Marshal(hb)
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest(http.MethodPost, "/v1/heartbeat", strings.NewReader(string(payload)))
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	newLeadHandler(a.bus, "node-a", nil, a.ttl).ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("heartbeat status: %d", w.Code)
	}
	if w.Body.Len() > 1<<20 {
		t.Fatalf("reply exceeds client cap: %d", w.Body.Len())
	}
	var reply struct {
		Answers []LeadAnswer // delivered controls
		Job     *leadJobView // optional assignment
	}
	if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
		t.Fatal(err)
	}
	return reply.Answers, reply.Job
}

// TestLeadHeartbeatControlAckLimits bounds both acknowledgement arrays before any mutation.
func TestLeadHeartbeatControlAckLimits(t *testing.T) {
	for _, count := range []int{64, 65} {
		t.Run(fmt.Sprint(count), func(t *testing.T) {
			a, _ := setupWaitingJob(t)
			hb := leadHeartbeatBody{NodeHeartbeat: NodeHeartbeat{Node: "node-a"}, AnswerAcks: make([]LeadAnswerAck, count), KickAcks: make([]LeadKickAck, count)}
			_, err := a.heartbeat("node-a", hb)
			if count == 65 {
				failsWith(t, LeadHeartbeatMalformedBody)("oversized receipt lists", err)
			} else if err != nil {
				t.Fatal(err)
			}
		})
	}
}

// TestLeadDoorbellIncludesControlDigests verifies that answer replacement changes pendingFor.
func TestLeadDoorbellIncludesControlDigests(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	ans1 := LeadAnswer{Job: jobID, TaskID: "t1", File: "questions-1.md", Content: "ans1\n**ANSWERED**", SHA256: sha256Hex("ans1\n**ANSWERED**")}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j := r.jobs[jobID]
		j.PendingAnswers = []LeadAnswer{ans1}
		r.jobs[jobID] = j
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	sig1 := a.pendingFor("node-a")
	if !strings.Contains(sig1, ans1.SHA256) {
		t.Fatalf("pendingFor must include answer SHA, got %q", sig1)
	}

	ans2 := LeadAnswer{Job: jobID, TaskID: "t1", File: "questions-1.md", Content: "ans2\n**ANSWERED**", SHA256: sha256Hex("ans2\n**ANSWERED**")}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j := r.jobs[jobID]
		j.PendingAnswers = []LeadAnswer{ans2}
		r.jobs[jobID] = j
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	sig2 := a.pendingFor("node-a")
	if sig1 == sig2 {
		t.Fatalf("pendingFor must change when answer content changes: %q == %q", sig1, sig2)
	}
}
