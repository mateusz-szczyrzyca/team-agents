package main

import (
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"
)

// Pending acceptance roadmap for the multi-instance jobs (task 20261023-151549-openrouter-haiku).
// Each test is a skipped placeholder for behaviour that a later round implements. A round removes
// the skips of its own tests and turns them into RED/GREEN tests. Nothing here may depend on code
// that does not exist yet; the names are the contract.

// TestLeadRegistrySubmitIsIdempotent: resubmitting the same job ID returns the existing job.
func TestLeadRegistrySubmitIsIdempotent(t *testing.T) {
	r := newLeadRegistry()
	first := regSubmit(t, r, regRequest("job-1"), regAt(0))
	again := regSubmit(t, r, regRequest("job-1"), regAt(5))
	if again.Seq != first.Seq || first.State != JobQueued {
		t.Fatalf("resubmission = %+v, want the queued job %+v", again, first)
	}
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(6))
	if got == nil || got.Request.ID != "job-1" {
		t.Fatalf("node-a got %+v, want job-1", got)
	}
	if extra := regHeartbeat(t, r, regBeat("node-b", nil, FreeAgent{Name: "beta", Tier: TierNormal}), regAt(7)); extra != nil {
		t.Fatalf("resubmission created a second queued job: node-b got %+v", extra)
	}
}

// TestLeadRegistryHeartbeatAssignsOneJobAtomically: one heartbeat gets at most one job, never twice.
func TestLeadRegistryHeartbeatAssignsOneJobAtomically(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	regSubmit(t, r, regRequest("job-2"), regAt(0))
	alpha := FreeAgent{Name: "alpha", Tier: TierNormal}
	beta := FreeAgent{Name: "beta", Tier: TierNormal}

	first := regHeartbeat(t, r, regBeat("node-a", nil, alpha, beta), regAt(1))
	if first == nil || first.Request.ID != "job-1" || first.State != JobAssigned {
		t.Fatalf("node-a with two free agents got %+v, want exactly job-1 assigned", first)
	}
	second := regHeartbeat(t, r, regBeat("node-b", nil, alpha), regAt(2))
	if second == nil || second.Request.ID != "job-2" {
		t.Fatalf("node-b got %+v, want job-2 (job-1 is already assigned)", second)
	}
	if third := regHeartbeat(t, r, regBeat("node-c", nil, alpha), regAt(3)); third != nil {
		t.Fatalf("node-c got %+v, but no queued job is left", third)
	}
}

// TestLeadRegistryRoutesByCapabilityAndAgent: a job goes only to a node with all required capabilities and a free agent of its tier.
func TestLeadRegistryRoutesByCapabilityAndAgent(t *testing.T) {
	normal := FreeAgent{Name: "alpha", Tier: TierNormal}

	t.Run("every required capability", func(t *testing.T) {
		r := newLeadRegistry()
		req := regRequest("job-1")
		req.Requires = []string{"os:linux", "hw:rp2350"}
		regSubmit(t, r, req, regAt(0))
		if got := regHeartbeat(t, r, regBeat("node-a", []string{"os:linux"}, normal), regAt(1)); got != nil {
			t.Fatalf("node-a without hw:rp2350 got %+v", got)
		}
		got := regHeartbeat(t, r, regBeat("node-b", []string{"os:linux", "hw:rp2350"}, normal), regAt(2))
		if got == nil || got.Node != "node-b" {
			t.Fatalf("node-b with every capability got %+v, want job-1", got)
		}
	})

	t.Run("agent tier", func(t *testing.T) {
		r := newLeadRegistry()
		req := regRequest("job-1")
		req.Selector = string(TierWorker)
		regSubmit(t, r, req, regAt(0))
		if got := regHeartbeat(t, r, regBeat("node-a", nil, normal), regAt(1)); got != nil {
			t.Fatalf("node-a with only a normal agent free got %+v for a worker job", got)
		}
		worker := FreeAgent{Name: "gamma", Tier: TierWorker}
		if got := regHeartbeat(t, r, regBeat("node-b", nil, worker), regAt(2)); got == nil || got.Request.ID != "job-1" {
			t.Fatalf("node-b with a free worker got %+v, want job-1", got)
		}
	})

	t.Run("agent name", func(t *testing.T) {
		r := newLeadRegistry()
		req := regRequest("job-1")
		req.Selector = "alpha"
		regSubmit(t, r, req, regAt(0))
		if got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "beta", Tier: TierNormal}), regAt(1)); got != nil {
			t.Fatalf("node-a without agent alpha got %+v", got)
		}
		if got := regHeartbeat(t, r, regBeat("node-b", nil, normal), regAt(2)); got == nil || got.Request.ID != "job-1" {
			t.Fatalf("node-b with agent alpha free got %+v, want job-1", got)
		}
	})

	t.Run("target pin", func(t *testing.T) {
		r := newLeadRegistry()
		req := regRequest("job-1")
		req.Target = "node-b"
		regSubmit(t, r, req, regAt(0))
		if got := regHeartbeat(t, r, regBeat("node-a", nil, normal), regAt(1)); got != nil {
			t.Fatalf("node-a got a job pinned to node-b: %+v", got)
		}
		if got := regHeartbeat(t, r, regBeat("node-b", nil, normal), regAt(2)); got == nil || got.Node != "node-b" {
			t.Fatalf("node-b got %+v, want the pinned job", got)
		}
	})

	t.Run("oldest queued job first", func(t *testing.T) {
		r := newLeadRegistry()
		regSubmit(t, r, regRequest("job-1"), regAt(0))
		regSubmit(t, r, regRequest("job-2"), regAt(1))
		if got := regHeartbeat(t, r, regBeat("node-a", nil, normal), regAt(2)); got == nil || got.Request.ID != "job-1" {
			t.Fatalf("node-a got %+v, want the oldest job-1", got)
		}
	})
}

// TestLeadRegistryWaitReasonNamesBlocker: a queued job says why it waits ("no node with hw:rp2350", "no free normal agent").
func TestLeadRegistryWaitReasonNamesBlocker(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-hw")
	req.Requires = []string{"hw:rp2350"}
	regSubmit(t, r, req, regAt(0))

	const noNode = "no node with hw:rp2350"
	if got := r.waitReason("job-hw", regAt(1), regTTL); got != noNode {
		t.Fatalf("no node at all: reason = %q, want %q", got, noNode)
	}
	if got := regHeartbeat(t, r, regBeat("node-a", []string{"os:linux"}), regAt(2)); got != nil {
		t.Fatalf("node-a without hw got %+v", got)
	}
	if got := r.waitReason("job-hw", regAt(3), regTTL); got != noNode {
		t.Fatalf("live node without hw: reason = %q, want %q", got, noNode)
	}

	regHeartbeat(t, r, regBeat("node-b", []string{"hw:rp2350"}, FreeAgent{Name: "gamma", Tier: TierWorker}), regAt(4))
	if got := r.waitReason("job-hw", regAt(5), regTTL); got != "no free normal agent" {
		t.Fatalf("live node with hw but no free normal agent: reason = %q", got)
	}
	// Liveness boundary: node-b heartbeated at 4s; 59s later it is still live, 60s later (ttl) it is offline.
	if got := r.waitReason("job-hw", regAt(63), regTTL); got != "no free normal agent" {
		t.Fatalf("node 59s old: reason = %q, want it still live", got)
	}
	if got := r.waitReason("job-hw", regAt(64), regTTL); got != noNode {
		t.Fatalf("node exactly ttl old: reason = %q, want %q (offline at ttl)", got, noNode)
	}

	if got := r.waitReason("job-hw", regAt(200), regTTL); got != noNode {
		t.Fatalf("every node stale: reason = %q, want %q (stale nodes do not count)", got, noNode)
	}
}

// TestLeadRegistryRejectsReportFromOtherNode: a state report from a node that is not assigned is refused with its cause.
func TestLeadRegistryRejectsReportFromOtherNode(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1, so there is no assignment to test")
	}

	other := r.checkAssignment("job-1", "node-b", got.Assignment)
	failsWith(t, LeadReportUnauthorized)("node-b reports node-a's job", other)
	var typed *LeadReportError
	if !errors.As(error(other), &typed) || typed.Cause != LeadReportUnauthorized {
		t.Fatalf("errors.As did not find the unauthorized cause: %v", other)
	}

	if err := r.checkAssignment("job-1", "node-a", got.Assignment); err != nil {
		t.Fatalf("assigned node-a refused: %v", err)
	}
	failsWith(t, LeadReportStaleAssignment)("node-a with a superseded assignment", r.checkAssignment("job-1", "node-a", got.Assignment+1))
	failsWith(t, LeadReportNoSuchJob)("report about an unknown job", r.checkAssignment("job-x", "node-a", got.Assignment))
}

// TestLeadRegistryRejectsSkippedTransition: assigned straight to done is refused; running must come first.
func TestLeadRegistryRejectsSkippedTransition(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}
	_, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobDone,
		Result:     "finished prematurely",
	}, regAt(2))
	failsWith(t, LeadReportInvalidState)("assigned straight to done", err)

	_, errWaiting := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
	}, regAt(2))
	failsWith(t, LeadReportInvalidState)("assigned straight to waiting", errWaiting)

	running, errRun := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
		Branch:     "branch-1",
	}, regAt(3))
	if errRun != nil {
		t.Fatalf("assigned to running failed: %v", errRun)
	}
	if running.State != JobRunning {
		t.Fatalf("state = %s, want running", running.State)
	}

	done, errDone := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobDone,
		Result:     "success",
	}, regAt(4))
	if errDone != nil {
		t.Fatalf("running to done failed: %v", errDone)
	}
	if done.State != JobDone {
		t.Fatalf("state = %s, want done", done.State)
	}
}

// TestLeadRegistryOfflineUnstartedJobRequeued: an offline node's assigned but unacknowledged job goes back to the queue.
func TestLeadRegistryOfflineUnstartedJobRequeued(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil || got.State != JobAssigned {
		t.Fatalf("heartbeat = %+v, want job-1 assigned", got)
	}
	requeued := r.requeueOffline(regAt(100), regTTL)
	if len(requeued) != 1 || requeued[0] != "job-1" {
		t.Fatalf("requeueOffline = %v, want [job-1]", requeued)
	}
	job, errGet := r.get("job-1")
	if errGet != nil {
		t.Fatalf("get job-1: %v", errGet)
	}
	if job.State != JobQueued || job.Node != "" {
		t.Fatalf("requeued job = %+v, want queued with no node", job)
	}
	taken := regHeartbeat(t, r, regBeat("node-b", nil, FreeAgent{Name: "beta", Tier: TierNormal}), regAt(101))
	if taken == nil || taken.Request.ID != "job-1" || taken.Node != "node-b" {
		t.Fatalf("node-b heartbeat = %+v, want job-1 assigned to node-b", taken)
	}
}

// TestLeadRegistryOfflineStartedJobNeverRequeued: an acknowledged running job is never reassigned while its node is offline.
func TestLeadRegistryOfflineStartedJobNeverRequeued(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}
	_, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
	}, regAt(2))
	if err != nil {
		t.Fatalf("start report: %v", err)
	}
	requeued := r.requeueOffline(regAt(100), regTTL)
	if len(requeued) != 0 {
		t.Fatalf("requeued started jobs: %v, want none", requeued)
	}
	job, errGet := r.get("job-1")
	if errGet != nil {
		t.Fatalf("get job-1: %v", errGet)
	}
	if job.State != JobRunning {
		t.Fatalf("job state = %s, want running", job.State)
	}
	if !r.nodeOffline("node-a", regAt(100), regTTL) {
		t.Fatal("node-a should be offline at t=100")
	}
	other := regHeartbeat(t, r, regBeat("node-b", nil, FreeAgent{Name: "beta", Tier: TierNormal}), regAt(101))
	if other != nil {
		t.Fatalf("node-b got %+v, want nil", other)
	}
}

// TestLeadRegistryCancelQueuedIsImmediate: cancelling a queued job stops it at once.
func TestLeadRegistryCancelQueuedIsImmediate(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("cancel-me"), regAt(0))
	stopped, err := r.cancel("cancel-me", regAt(1))
	if err != nil {
		t.Fatal(err)
	}
	if stopped.State != JobStopped || stopped.StopRequested || len(stopped.History) != 2 || !stopped.History[1].At.Equal(regAt(1).Time) {
		t.Fatalf("queued cancellation = %+v, want immediate stopped transition", stopped)
	}
	again, retryErr := r.cancel("cancel-me", regAt(2))
	if retryErr != nil || len(again.History) != len(stopped.History) {
		t.Fatalf("cancel retry changed history: %+v, %v", again, retryErr)
	}
	if got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(3)); got != nil {
		t.Fatalf("cancelled job was assigned: %+v", got)
	}
}

// TestLeadRegistryCancelActiveWaitsForNodeConfirmation: an active job stays in its state until the node reports stopped.
func TestLeadRegistryCancelActiveWaitsForNodeConfirmation(t *testing.T) {
	for _, state := range []JobState{JobRunning, JobWaiting} {
		t.Run(string(state), func(t *testing.T) {
			b := newBus(t)
			storeRun(t, b, regAt(2), func(r *leadRegistry) { cancelActiveFixture(t, r, state) })
			err := transactJobs(b, LeadCancelStorage, regAt(3), func(r *leadRegistry) *LeadCancelError {
				pending, err := r.cancel("cancel-me", regAt(3))
				if err != nil {
					return err
				}
				if pending.State != state || !pending.StopRequested {
					t.Fatalf("active cancellation = %+v, want same state and pending stop", pending)
				}
				return nil
			})
			if err != nil {
				t.Fatal(err)
			}
			storeRun(t, &Bus{dir: b.dir}, regAt(4), func(r *leadRegistry) {
				job := r.jobs["cancel-me"]
				if job.State != state || !job.StopRequested {
					t.Fatalf("restart lost pending stop: %+v", job)
				}
				before := len(job.History)
				pending, err := r.cancel("cancel-me", regAt(4))
				if err != nil || len(pending.History) != before || !pending.StopRequested {
					t.Fatalf("repeat active cancellation changed history: %+v, %v", pending, err)
				}
			})
		})
	}
}

// TestLeadRegistryReportHistoryKeepsFirstStart: cumulative report history keeps the first acknowledged start.
func TestLeadRegistryReportHistoryKeepsFirstStart(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}
	firstStart := regAt(2)
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
		At:         firstStart,
	}, firstStart); err != nil {
		t.Fatalf("first start: %v", err)
	}

	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
		At:         regAt(10),
	}, regAt(10)); err != nil {
		t.Fatalf("waiting 1: %v", err)
	}
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
		At:         regAt(20),
	}, regAt(20)); err != nil {
		t.Fatalf("waiting 2: %v", err)
	}
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		At:         regAt(30),
	}, regAt(30)); err != nil {
		t.Fatalf("resume running: %v", err)
	}

	job, errGet := r.get("job-1")
	if errGet != nil {
		t.Fatalf("get job-1: %v", errGet)
	}
	var runningTimes []Timestamp
	var waitingCount int
	for _, tr := range job.History {
		if tr.State == JobRunning {
			runningTimes = append(runningTimes, tr.At)
		}
		if tr.State == JobWaiting {
			waitingCount++
		}
	}
	if len(runningTimes) < 2 || runningTimes[0] != firstStart {
		t.Fatalf("running times = %v, want first start %v preserved", runningTimes, firstStart)
	}
	if waitingCount != 2 {
		t.Fatalf("waiting count = %d, want 2 repeated waiting reports", waitingCount)
	}
}

// TestLeadRegistryRetryIsIdempotent: an identical report retry is accepted; a conflicting terminal revision is refused.
func TestLeadRegistryRetryIsIdempotent(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
	}, regAt(2)); err != nil {
		t.Fatalf("start: %v", err)
	}
	terminalRep := JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobDone,
		TaskID:     "task-1",
		Result:     "success",
		At:         regAt(3),
	}
	done, err := r.report(terminalRep, regAt(3))
	if err != nil {
		t.Fatalf("terminal report: %v", err)
	}
	if done.State != JobDone {
		t.Fatalf("state = %s, want done", done.State)
	}

	again, errRetry := r.report(terminalRep, regAt(4))
	if errRetry != nil {
		t.Fatalf("identical retry refused: %v", errRetry)
	}
	if again.State != JobDone || again.Result != "success" {
		t.Fatalf("retry job = %+v, want done with success", again)
	}

	conflict := JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobFailed,
		TaskID:     "task-1",
		Result:     "conflicting failure",
		At:         regAt(5),
	}
	_, errConflict := r.report(conflict, regAt(5))
	failsWith(t, LeadReportTerminalConflict)("conflicting terminal report", errConflict)

	diffResult := terminalRep
	diffResult.Result = "different result"
	_, errDiff := r.report(diffResult, regAt(6))
	failsWith(t, LeadReportTerminalConflict)("different terminal result", errDiff)
}

// TestLeadHTTPIdentityLoopbackIsLeadNode: a connection from 127.0.0.1 or ::1 is the lead's own node.
func TestLeadHTTPIdentityLoopbackIsLeadNode(t *testing.T) {
	calls := 0
	stubWhois(t, func(_, _ string) (*leadWhoisResponse, *LeadWhoisError) {
		calls++
		return nil, opErr(LeadWhoisTailscaleFailed, "whois should not be consulted for loopback")
	})

	for _, addr := range []string{"127.0.0.1:54321", "127.0.0.1", "[::1]:54321", "::1"} {
		id, err := identifyLeadPeer(addr, "lead-node", []string{"other-node"})
		if err != nil {
			t.Fatalf("identifyLeadPeer(%q) error: %v", addr, err)
		}
		if !id.Local || id.Node != "lead-node" {
			t.Fatalf("identifyLeadPeer(%q) = %+v, want {Node: lead-node, Local: true}", addr, id)
		}
	}
	if calls != 0 {
		t.Fatalf("tailscale whois called %d times, want 0 for local loopback", calls)
	}
}

// TestLeadHTTPIdentityAllowsListedDeviceByNameAndTag: LEAD_NODES entries match device name or tag:….
func TestLeadHTTPIdentityAllowsListedDeviceByNameAndTag(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		if ip == "100.64.0.10" {
			return &leadWhoisResponse{
				Node: leadWhoisNode{
					ComputedName: "worker-1",
					Name:         "worker-1.example.ts.net.",
					Tags:         []string{"tag:ci", "tag:runners"},
				},
			}, nil
		}
		return nil, opErr(LeadWhoisTailscaleFailed, "unknown ip %s", ip)
	})

	allowedCases := [][]string{
		{"worker-1"},
		{"WORKER-1"},
		{"worker-1.example.ts.net"},
		{"tag:ci"},
		{"TAG:RUNNERS"},
		{"other-node", "tag:ci"},
	}
	for _, allowed := range allowedCases {
		id, err := identifyLeadPeer("100.64.0.10:4321", "lead-node", allowed)
		if err != nil {
			t.Fatalf("identifyLeadPeer(allowed=%v) unexpected error: %v", allowed, err)
		}
		if id.Local || id.Node != "worker-1" {
			t.Fatalf("identifyLeadPeer(allowed=%v) = %+v, want {Node: worker-1, Local: false}", allowed, id)
		}
	}
}

// TestLeadHTTPIdentityDeniesUnlistedDevice: a device not in LEAD_NODES is refused.
func TestLeadHTTPIdentityDeniesUnlistedDevice(t *testing.T) {
	stubWhois(t, func(_, ip string) (*leadWhoisResponse, *LeadWhoisError) {
		if ip == "100.64.0.20" {
			return &leadWhoisResponse{
				Node: leadWhoisNode{
					ComputedName: "untrusted-box",
					Name:         "untrusted-box.example.ts.net.",
					Tags:         []string{"tag:external"},
				},
			}, nil
		}
		return nil, opErr(LeadWhoisTailscaleFailed, "unknown ip %s", ip)
	})

	id, err := identifyLeadPeer("100.64.0.20:5555", "lead-node", []string{"worker-1", "tag:ci"})
	if err == nil {
		t.Fatalf("identifyLeadPeer returned id %+v, want refusal", id)
	}
	var whoisErr *LeadWhoisError
	if !errors.As(err, &whoisErr) {
		t.Fatalf("error %v is not *LeadWhoisError", err)
	}
	if whoisErr.Cause != LeadWhoisUntrustedNode {
		t.Fatalf("cause = %v, want LeadWhoisUntrustedNode", whoisErr.Cause)
	}
}

// TestLeadHTTPIdentityFailsWhenWhoisNotStubbed: the test whois fake fails when a test forgets to stub it.
func TestLeadHTTPIdentityFailsWhenWhoisNotStubbed(t *testing.T) {
	defer func() {
		r := recover()
		if r == nil {
			t.Fatal("expected unstubbed whois to panic, but it returned normally")
		}
		msg := fmt.Sprint(r)
		if !strings.Contains(msg, "without a stub") {
			t.Fatalf("panic message %q does not mention missing stub", msg)
		}
	}()
	_, _ = identifyLeadPeer("100.64.0.30:5555", "lead-node", []string{"any"})
}

// TestNodeCapabilitiesRejectUnknownNamespace: an unknown capability namespace stops the node with a clear error.
func TestNodeCapabilitiesRejectUnknownNamespace(t *testing.T) {
	a := &leadAPI{bus: newBus(t)}
	_, err := a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: NodeHeartbeat{Capabilities: []string{"sign:key"}}})
	failsWith(t, LeadHeartbeatInvalidCapabilities)("HTTP boundary capability validation", err)
}

// TestNodeTwoHomesRecoverAfterLeadRestart: a node keeps its job running and reports after the lead returns.
func TestNodeTwoHomesRecoverAfterLeadRestart(t *testing.T) {
	assertNodeTransportRecovery(t)
}

// TestNodeCheckoutFailureFailsJobWithReason: a failed clone or worktree fails the job with a typed reason; nothing is marked started.
func TestNodeCheckoutFailureFailsJobWithReason(t *testing.T) {
	stubNodeGit(t, func(_ string, _ ...string) ([]byte, *ProcessError) {
		return nil, processErr("git", nil, "fetch refused by remote")
	})
	registry := newLeadRegistry()
	req := regRequest("checkout-failure")
	regSubmit(t, registry, req, regAt(0))
	assigned := regHeartbeat(t, registry, regBeat("node-a", nil, FreeAgent{Name: "builder", Tier: TierNormal}), regAt(1))
	if assigned == nil {
		t.Fatal("job was not assigned")
	}
	_, err := nodeCheckout(map[string]string{req.Repository: t.TempDir()}, NodeCheckoutRequest{Repository: req.Repository, BaseRef: req.BaseRef})
	failsWith(t, NodeCheckoutFetchFailed)("checkout failure", err)
	if err == nil {
		return
	}
	failed, reportErr := registry.report(JobReport{ID: req.ID, Node: "node-a", Assignment: assigned.Assignment, State: JobFailed, Result: err.Error()}, regAt(2))
	if reportErr != nil {
		t.Fatal(reportErr)
	}
	if failed.State != JobFailed || failed.Result == "" {
		t.Fatalf("outcome=%+v", failed)
	}
	for _, transition := range failed.History {
		if transition.State == JobRunning {
			t.Fatal("checkout failure recorded as running")
		}
	}
}

// TestNodeRefusesRepositoryOutsideAllowList: a job naming a repository not in NODE_REPOS is refused.
func TestNodeRefusesRepositoryOutsideAllowList(t *testing.T) {
	_, err := nodeCheckout(map[string]string{"allowed": t.TempDir()}, NodeCheckoutRequest{Repository: "forbidden", BaseRef: "main"})
	failsWith(t, NodeCheckoutRepositoryNotAllowed)("repository allow-list", err)
	_, emptyErr := nodeCheckout(map[string]string{"empty": ""}, NodeCheckoutRequest{Repository: "empty", BaseRef: "main"})
	failsWith(t, NodeCheckoutRepositoryNotAllowed)("empty allow-list path", emptyErr)
}

// TestCLIRunNeedSubmitsJob: agent run --need and --on submit a job through the lead.
func TestCLIRunNeedSubmitsJob(t *testing.T) {
	b := newBus(t)
	a := &leadAPI{bus: b, ttl: time.Minute}

	// 1. Submit a job with capability requirements (--need).
	req := regRequest("cli-need-job")
	req.Requires = []string{"hw:rp2350"}
	req.Requester = "human"
	req.Selector = "worker"
	job, err := a.submit("lead-node", true, req)
	if err != nil {
		t.Fatalf("submit --need failed: %v", err)
	}
	if job.State != JobQueued || len(job.Request.Requires) != 1 || job.Request.Requires[0] != "hw:rp2350" {
		t.Fatalf("unexpected queued job: %+v", job)
	}

	// Heartbeat from a node without the capability must not receive the job.
	otherBeat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-plain", []string{"os:linux"}, FreeAgent{Name: "w1", Tier: TierWorker})}
	assigned, beatErr := a.heartbeat("node-plain", otherBeat)
	if beatErr != nil {
		t.Fatal(beatErr)
	}
	if assigned != nil {
		t.Fatalf("node-plain received job without required capability: %+v", assigned)
	}

	// Diagnosis explains why the job waits.
	view, getErr := a.get("cli-need-job")
	if getErr != nil {
		t.Fatal(getErr)
	}
	if view.WaitReason != "no node with hw:rp2350" {
		t.Fatalf("waitReason = %q, want 'no node with hw:rp2350'", view.WaitReason)
	}

	// Heartbeat from a node declaring the capability and a free worker receives the job.
	hwBeat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-hw", []string{"os:linux", "hw:rp2350"}, FreeAgent{Name: "w1", Tier: TierWorker})}
	assigned, beatErr = a.heartbeat("node-hw", hwBeat)
	if beatErr != nil {
		t.Fatal(beatErr)
	}
	if assigned == nil || assigned.Request.ID != "cli-need-job" {
		t.Fatalf("node-hw failed to receive job: %+v", assigned)
	}

	// 2. Submit a job pinned to a node (--on).
	reqPin := regRequest("cli-on-job")
	reqPin.Target = "node-target"
	reqPin.Requester = "human"
	reqPin.Selector = "normal"
	jobPin, errPin := a.submit("lead-node", true, reqPin)
	if errPin != nil {
		t.Fatalf("submit --on failed: %v", errPin)
	}
	if jobPin.State != JobQueued || jobPin.Request.Target != "node-target" {
		t.Fatalf("unexpected pinned job: %+v", jobPin)
	}

	// Other node does not receive it.
	assigned, beatErr = a.heartbeat("node-other", leadHeartbeatBody{NodeHeartbeat: regBeat("node-other", nil, FreeAgent{Name: "alpha", Tier: TierNormal})})
	if beatErr != nil {
		t.Fatal(beatErr)
	}
	if assigned != nil {
		t.Fatalf("node-other received pinned job: %+v", assigned)
	}

	// Target node receives it.
	assigned, beatErr = a.heartbeat("node-target", leadHeartbeatBody{NodeHeartbeat: regBeat("node-target", nil, FreeAgent{Name: "beta", Tier: TierNormal})})
	if beatErr != nil {
		t.Fatal(beatErr)
	}
	if assigned == nil || assigned.Request.ID != "cli-on-job" {
		t.Fatalf("node-target failed to receive pinned job: %+v", assigned)
	}

	// 3. Invalid capability namespace is rejected.
	reqBad := regRequest("cli-bad-caps")
	reqBad.Requires = []string{"invalid_namespace:x"}
	reqBad.Requester = "human"
	_, errBad := a.submit("lead-node", true, reqBad)
	failsWith(t, LeadSubmitInvalidCapabilities)("invalid capability namespace", errBad)
}

// TestDelegateRequiresCreatesJob: a delegate request with requires the local node lacks becomes a job on the lead.
func TestDelegateRequiresCreatesJob(t *testing.T) {
	b := newBus(t)
	remoteHelperFixture(t, b)
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) { return nil, nil })
	req, err := prepareRemoteHelper(b, 1, "node-a")
	if err != nil {
		t.Fatal(err)
	}
	r := newLeadRegistry()
	job, submitErr := r.submit(req, regAt(2))
	if submitErr != nil {
		t.Fatal(submitErr)
	}
	if job.Request.BaseSHA != regSHA || job.Request.Requester != "member@node-a" || job.State != JobQueued {
		t.Fatalf("wrong delegated job: %+v", job)
	}
}

// TestDelegateResultReturnsHelperMessage: the finished job reaches the member as the ordinary helper-result message.
func TestDelegateResultReturnsHelperMessage(t *testing.T) {
	b := newBus(t)
	remoteHelperFixture(t, b)
	if err := finishRemoteHelper(b, 1, remoteHelperResult{State: JobDone, Node: "node-b", Branch: "agents/result", Result: "build passed"}); err != nil {
		t.Fatal(err)
	}
	msgs, err := b.messages()
	if err != nil || len(msgs) != 1 {
		t.Fatalf("missing helper result: %+v %v", msgs, err)
	}
	if msgs[0].To != "member" || !strings.Contains(msgs[0].Text, "build passed") {
		t.Fatalf("wrong helper result: %+v", msgs[0])
	}
}
