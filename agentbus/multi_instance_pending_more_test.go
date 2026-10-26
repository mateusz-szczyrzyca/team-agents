package main

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// Companion roadmap for cases that the design (docs/multi-instance.md) requires and that
// multi_instance_pending_test.go does not yet name. Like that file, every test is a skipped
// placeholder; the owning round replaces the skip with executable RED/GREEN tests.

// TestLeadRegistryExpiredAssignmentCannotStart: an expired assignment is refused even when the same node gets a new assignment for the same job.
func TestLeadRegistryExpiredAssignmentCannotStart(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	first := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if first == nil {
		t.Fatal("node-a did not get job-1")
	}
	requeued := r.requeueOffline(regAt(100), regTTL)
	if len(requeued) != 1 || requeued[0] != "job-1" {
		t.Fatalf("requeueOffline = %v, want [job-1]", requeued)
	}
	second := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(101))
	if second == nil || second.Assignment == first.Assignment {
		t.Fatalf("second assignment = %+v, want new assignment", second)
	}
	_, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: first.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
	}, regAt(102))
	failsWith(t, LeadReportStaleAssignment)("expired assignment cannot start", err)
}

// TestLeadRegistryCancelUnacknowledgedAssignmentIsImmediate: cancelling an assigned but unacknowledged job stops it at once.
func TestLeadRegistryCancelUnacknowledgedAssignmentIsImmediate(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("cancel-me"), regAt(0))
	assigned := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if assigned == nil {
		t.Fatal("job not assigned")
	}
	stopped, err := r.cancel("cancel-me", regAt(2))
	if err != nil {
		t.Fatal(err)
	}
	if stopped.State != JobStopped || stopped.StopRequested || stopped.Assignment != assigned.Assignment || stopped.Node != assigned.Node {
		t.Fatalf("unstarted cancellation = %+v, want stopped with original ownership", stopped)
	}
	if got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(3)); got != nil {
		t.Fatalf("cancelled assignment offered again: %+v", got)
	}
}

// TestLeadRegistryFinishedJobsKeptThirtyDays: finished jobs stay visible for 30 days and are then removed.
func TestLeadRegistryFinishedJobsKeptThirtyDays(t *testing.T) {
	b := newBus(t)
	finished := regAt(10)
	for _, state := range []JobState{JobDone, JobFailed, JobStopped} {
		storeRun(t, b, regAt(0), func(r *leadRegistry) {
			job := regSubmit(t, r, regRequest(string(state)), regAt(0))
			r.jobs[job.Request.ID].State = state
			r.jobs[job.Request.ID].History = append(r.jobs[job.Request.ID].History, JobTransition{State: state, At: finished})
		})
	}
	storeRun(t, b, regAt(0), func(r *leadRegistry) { regSubmit(t, r, regRequest("unfinished"), regAt(0)) })
	before := Timestamp{Time: finished.Add(30*24*time.Hour - time.Nanosecond)}
	storeRun(t, b, before, func(r *leadRegistry) {
		if len(r.jobs) != 4 {
			t.Fatalf("before expiry: got %d jobs, want 4", len(r.jobs))
		}
	})
	storeRun(t, b, Timestamp{Time: finished.Add(30 * 24 * time.Hour)}, func(r *leadRegistry) {
		if len(r.jobs) != 1 || r.jobs["unfinished"] == nil {
			t.Fatalf("expiry retained wrong jobs: %+v", r.jobs)
		}
		if len(r.order) != 1 || r.order[0] != "unfinished" {
			t.Fatalf("expiry left stale ordering: %v", r.order)
		}
	})
}

// TestLeadRegistryAbsentJobsFileIsEmpty: a missing jobs.json is an empty registry, not an error.
func TestLeadRegistryAbsentJobsFileIsEmpty(t *testing.T) {
	b := newBus(t)
	called := false
	storeRun(t, b, regAt(0), func(r *leadRegistry) {
		called = true
		if len(r.jobs) != 0 || len(r.nodes) != 0 {
			t.Fatal("absent file did not load empty registry")
		}
		regSubmit(t, r, regRequest("first"), regAt(0))
	})
	if !called {
		t.Fatal("registry operation was not invoked")
	}
	storeRun(t, &Bus{dir: b.dir}, regAt(1), func(r *leadRegistry) {
		if r.jobs["first"] == nil {
			t.Fatal("job missing after reopening store")
		}
	})
	if _, err := os.Stat(b.path("state.json")); !os.IsNotExist(err) {
		t.Fatalf("job transaction touched ordinary state: %v", err)
	}
}

// TestLeadRegistryCorruptJobsFileFailsStorage: a corrupt jobs.json fails with a typed storage cause and is never reset silently.
func TestLeadRegistryCorruptJobsFileFailsStorage(t *testing.T) {
	for _, data := range []string{"{broken", "", "null", "{}", `{"Version":2}`} {
		t.Run(fmt.Sprintf("%q", data), func(t *testing.T) {
			b := newBus(t)
			if err := os.WriteFile(b.path("jobs.json"), []byte(data), 0o600); err != nil {
				t.Fatal(err)
			}
			called := false
			err := transactJobs(b, LeadSubmitStorage, regAt(0), func(*leadRegistry) *LeadSubmitError { called = true; return nil })
			failsWith(t, LeadSubmitStorage)("corrupt registry", err)
			if called {
				t.Fatal("callback ran on corrupt storage")
			}
			after, readErr := os.ReadFile(b.path("jobs.json"))
			if readErr != nil || !bytes.Equal(after, []byte(data)) {
				t.Fatalf("corruption overwritten: %q, %v", after, readErr)
			}
		})
	}
}

// TestLeadRegistryStopRequestReturnedUntilAcknowledged: a durable stop request is returned in heartbeats until the node acknowledges it.
func TestLeadRegistryStopRequestReturnedUntilAcknowledged(t *testing.T) {
	assertHeartbeatStopDelivery(t)
}

// TestLeadRegistryLatestHeartbeatWinsForCapabilities: the capabilities of the latest heartbeat are the ones used for routing.
func TestLeadRegistryLatestHeartbeatWinsForCapabilities(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-hw")
	req.Requires = []string{"hw:rp2350"}
	regSubmit(t, r, req, regAt(0))
	alpha := FreeAgent{Name: "alpha", Tier: TierNormal}

	// The first heartbeat declares hw:rp2350 but has no free agent, so nothing is assigned.
	if got := regHeartbeat(t, r, regBeat("node-a", []string{"os:linux", "hw:rp2350"}), regAt(1)); got != nil {
		t.Fatalf("node-a without a free agent got %+v", got)
	}
	// The latest heartbeat drops hw:rp2350 and has a free agent: the old declaration must not route the job.
	if got := regHeartbeat(t, r, regBeat("node-a", []string{"os:linux"}, alpha), regAt(2)); got != nil {
		t.Fatalf("node-a whose latest heartbeat lacks hw:rp2350 got %+v", got)
	}
	if reason := r.waitReason("job-hw", regAt(3), regTTL); reason != "no node with hw:rp2350" {
		t.Fatalf("reason = %q, want the latest declaration to count", reason)
	}
	// And the reverse: a later heartbeat that declares hw:rp2350 gets the job.
	got := regHeartbeat(t, r, regBeat("node-a", []string{"os:linux", "hw:rp2350"}, alpha), regAt(4))
	if got == nil || got.Request.ID != "job-hw" {
		t.Fatalf("node-a with hw:rp2350 again got %+v, want job-hw", got)
	}
}

// TestLeadHTTPLoopbackRangeRequiresIdentity: 127.0.0.2 is not local authority; it needs an authenticated nonlocal node identity.
func TestLeadHTTPLoopbackRangeRequiresIdentity(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		if ip == "127.0.0.2" {
			return &leadWhoisResponse{
				Node: leadWhoisNode{
					ComputedName: "node2",
					Name:         "node2.example.ts.net.",
					Tags:         []string{"tag:runners"},
				},
			}, nil
		}
		return nil, opErr(LeadWhoisTailscaleFailed, "unknown ip %s", ip)
	})

	// 127.0.0.2 must not be treated as local authority.
	id, err := identifyLeadPeer("127.0.0.2:12345", "lead-node", []string{"node2"})
	if err != nil {
		t.Fatalf("identifyLeadPeer(127.0.0.2) unexpected error: %v", err)
	}
	if id.Local || id.Node != "node2" {
		t.Fatalf("identifyLeadPeer(127.0.0.2) = %+v, want {Node: node2, Local: false}", id)
	}

	// When 127.0.0.2 is not in allowed list, it is refused with LeadWhoisUntrustedNode.
	_, errDenied := identifyLeadPeer("127.0.0.2:12345", "lead-node", []string{"other-node"})
	if errDenied == nil {
		t.Fatal("expected 127.0.0.2 with unlisted device to fail, got nil")
	}
	var whoisErr *LeadWhoisError
	if !errors.As(errDenied, &whoisErr) {
		t.Fatalf("errDenied %v is not *LeadWhoisError", errDenied)
	}
	if whoisErr.Cause != LeadWhoisUntrustedNode {
		t.Fatalf("cause = %v, want LeadWhoisUntrustedNode", whoisErr.Cause)
	}
}

// TestLeadNotifiesOnFinalState: the lead notifies when a job becomes done, failed or stopped.
func TestLeadNotifiesOnFinalState(t *testing.T) {
	b := newBus(t)
	notifyDir := t.TempDir()
	a := &leadAPI{bus: b, ttl: time.Minute, notifyDir: notifyDir}

	// 1. Submit and run a job to done.
	req1 := regRequest("job-done-1")
	req1.Repository = "test-repo"
	job1, err := a.submit("node-a", true, req1)
	if err != nil {
		t.Fatal(err)
	}
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})}
	assigned, beatErr := a.heartbeat("node-a", beat)
	if beatErr != nil || assigned == nil {
		t.Fatalf("assignment failed: %v", beatErr)
	}

	at := Timestamp{time.Now().UTC()}
	repRunning := JobReport{ID: job1.Request.ID, Assignment: assigned.Assignment, State: JobRunning, Node: "node-a", Branch: "agents/alpha/job", At: at}
	if _, err := a.report("node-a", job1.Request.ID, repRunning); err != nil {
		t.Fatal(err)
	}

	// Intermediate running report must not trigger notification.
	entries, rerr := os.ReadDir(notifyDir)
	if rerr != nil {
		t.Fatal(rerr)
	}
	if len(entries) != 0 {
		t.Fatalf("expected 0 notifications for running job, got %d", len(entries))
	}

	// Report done.
	repDone := repRunning
	repDone.State = JobDone
	repDone.Result = "all tasks finished successfully"
	repDone.PRURL = "https://example.com/pr/1"
	repDone.At = Timestamp{at.Add(time.Second)}
	if _, err := a.report("node-a", job1.Request.ID, repDone); err != nil {
		t.Fatal(err)
	}

	// Verify notification file for done job.
	entries, dirErr := os.ReadDir(notifyDir)
	if dirErr != nil || len(entries) != 1 {
		t.Fatalf("expected 1 notification file for done job, got %d (err: %v)", len(entries), dirErr)
	}
	content, readErr := os.ReadFile(filepath.Join(notifyDir, entries[0].Name()))
	if readErr != nil {
		t.Fatal(readErr)
	}
	text := string(content)
	for _, expected := range []string{"test-repo", "job-done-1", "node-a", "agents/alpha/job", "done", "all tasks finished successfully", "https://example.com/pr/1"} {
		if !strings.Contains(text, expected) {
			t.Errorf("notification missing %q:\n%s", expected, text)
		}
	}

	// Duplicate terminal report must not write an additional notification.
	if _, err := a.report("node-a", job1.Request.ID, repDone); err != nil {
		t.Fatal(err)
	}
	entries, _ = os.ReadDir(notifyDir)
	if len(entries) != 1 {
		t.Fatalf("duplicate terminal report created extra notification: %d files", len(entries))
	}

	// 2. Job failing.
	req2 := regRequest("job-failed-1")
	req2.Repository = "test-repo"
	job2, err := a.submit("node-a", true, req2)
	if err != nil {
		t.Fatal(err)
	}
	assigned2, beatErr := a.heartbeat("node-a", beat)
	if beatErr != nil || assigned2 == nil {
		t.Fatalf("assignment 2 failed: %v", beatErr)
	}
	repFailed := JobReport{ID: job2.Request.ID, Assignment: assigned2.Assignment, State: JobFailed, Node: "node-a", Result: "build error: syntax", At: Timestamp{at.Add(2 * time.Second)}}
	if _, err := a.report("node-a", job2.Request.ID, repFailed); err != nil {
		t.Fatal(err)
	}
	entries, _ = os.ReadDir(notifyDir)
	if len(entries) != 2 {
		t.Fatalf("expected 2 notification files after failure, got %d", len(entries))
	}

	// 3. Job stopped while queued.
	req3 := regRequest("job-stopped-1")
	req3.Repository = "test-repo"
	job3, err := a.submit("node-a", true, req3)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.cancel(true, job3.Request.ID); err != nil {
		t.Fatal(err)
	}
	entries, _ = os.ReadDir(notifyDir)
	if len(entries) != 3 {
		t.Fatalf("expected 3 notification files after cancel, got %d", len(entries))
	}
}

// TestLeadRegistryConcurrentHeartbeatsRace: concurrent heartbeats and submits keep the registry consistent under -race.
func TestLeadRegistryConcurrentHeartbeatsRace(t *testing.T) {
	const jobs = 50
	const nodes = 60
	r := newLeadRegistry()

	// Every job is submitted twice, concurrently: the duplicate must be the same job.
	var wg sync.WaitGroup
	submitErr := make([]*LeadSubmitError, 2*jobs)
	for i := range 2 * jobs {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := r.submit(regRequest(fmt.Sprintf("job-%02d", i%jobs)), regAt(0)); err != nil {
				submitErr[i] = err
			}
		}()
	}
	wg.Wait()
	for i, err := range submitErr {
		if err != nil {
			t.Fatalf("concurrent submit %d refused: %v", i, err)
		}
	}

	// Every node heartbeats once, concurrently, with one free normal agent.
	heartbeats := make([]*Job, nodes)
	heartbeatErr := make([]*LeadHeartbeatError, nodes)
	for n := range nodes {
		wg.Add(1)
		go func() {
			defer wg.Done()
			h := regBeat(fmt.Sprintf("node-%02d", n), []string{"os:linux"}, FreeAgent{Name: "alpha", Tier: TierNormal})
			got, err := r.heartbeat(h, regAt(1))
			if err != nil {
				heartbeatErr[n] = err
			}
			heartbeats[n] = got
		}()
	}
	wg.Wait()

	assigned := map[string]bool{}
	assignments := map[uint64]bool{}
	for n, err := range heartbeatErr {
		if err != nil {
			t.Fatalf("node-%02d heartbeat refused: %v", n, err)
		}
		got := heartbeats[n]
		if got == nil {
			continue
		}
		if assigned[got.Request.ID] {
			t.Fatalf("job %s assigned twice", got.Request.ID)
		}
		if assignments[got.Assignment] || got.Assignment == 0 {
			t.Fatalf("assignment %d is not unique and positive", got.Assignment)
		}
		if got.Node != fmt.Sprintf("node-%02d", n) || got.State != JobAssigned {
			t.Fatalf("node-%02d got %+v, want its own assigned job", n, got)
		}
		assigned[got.Request.ID] = true
		assignments[got.Assignment] = true
	}
	if len(assigned) != jobs {
		t.Fatalf("%d jobs assigned, want all %d (60 nodes can take them)", len(assigned), jobs)
	}
}

// TestLeadHTTPListenerConcurrentRequestsRace: concurrent HTTP requests to the listener are race-free under -race.
func TestLeadHTTPListenerConcurrentRequestsRace(t *testing.T) {
	leadConcurrentRequests(t)
}
