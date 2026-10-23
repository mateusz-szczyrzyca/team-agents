package main

import (
	"errors"
	"testing"
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
	t.Skip("pending: transition table (assigned->running required before done), round 3")
}

// TestLeadRegistryOfflineUnstartedJobRequeued: an offline node's assigned but unacknowledged job goes back to the queue.
func TestLeadRegistryOfflineUnstartedJobRequeued(t *testing.T) {
	t.Skip("pending: offline node's assigned, unacknowledged job returns to queued, round 3")
}

// TestLeadRegistryOfflineStartedJobNeverRequeued: an acknowledged running job is never reassigned while its node is offline.
func TestLeadRegistryOfflineStartedJobNeverRequeued(t *testing.T) {
	t.Skip("pending: acknowledged running job is never requeued; shows node offline, round 3")
}

// TestLeadRegistryCancelQueuedIsImmediate: cancelling a queued job stops it at once.
func TestLeadRegistryCancelQueuedIsImmediate(t *testing.T) {
	t.Skip("pending: cancel of a queued or unacknowledged job is immediate, round 3")
}

// TestLeadRegistryCancelActiveWaitsForNodeConfirmation: an active job stays in its state until the node reports stopped.
func TestLeadRegistryCancelActiveWaitsForNodeConfirmation(t *testing.T) {
	t.Skip("pending: active cancel is a durable stop request until the node reports stopped, round 3")
}

// TestLeadRegistryReportHistoryKeepsFirstStart: cumulative report history keeps the first acknowledged start.
func TestLeadRegistryReportHistoryKeepsFirstStart(t *testing.T) {
	t.Skip("pending: repeated waiting reports and the first acknowledged start survive compaction, round 3")
}

// TestLeadRegistryRetryIsIdempotent: an identical report retry is accepted; a conflicting terminal revision is refused.
func TestLeadRegistryRetryIsIdempotent(t *testing.T) {
	t.Skip("pending: identical retries accepted, conflicting terminal revisions refused, round 3")
}

// TestLeadHTTPIdentityLoopbackIsLeadNode: a connection from 127.0.0.1 or ::1 is the lead's own node.
func TestLeadHTTPIdentityLoopbackIsLeadNode(t *testing.T) {
	t.Skip("pending: loopback peers are the lead's node, whois not consulted, round 4")
}

// TestLeadHTTPIdentityAllowsListedDeviceByNameAndTag: LEAD_NODES entries match device name or tag:….
func TestLeadHTTPIdentityAllowsListedDeviceByNameAndTag(t *testing.T) {
	t.Skip("pending: devices allowed by short name or tag from LEAD_NODES, round 4")
}

// TestLeadHTTPIdentityDeniesUnlistedDevice: a device not in LEAD_NODES is refused.
func TestLeadHTTPIdentityDeniesUnlistedDevice(t *testing.T) {
	t.Skip("pending: unlisted tailnet device refused, cause checked with errors.As, round 4")
}

// TestLeadHTTPIdentityFailsWhenWhoisNotStubbed: the test whois fake fails when a test forgets to stub it.
func TestLeadHTTPIdentityFailsWhenWhoisNotStubbed(t *testing.T) {
	t.Skip("pending: unstubbed whois fails the test, as in live/, round 4")
}

// TestNodeCapabilitiesRejectUnknownNamespace: an unknown capability namespace stops the node with a clear error.
func TestNodeCapabilitiesRejectUnknownNamespace(t *testing.T) {
	t.Skip("pending: capabilities outside os, arch, build, test, hw, net are refused, round 4")
}

// TestNodeTwoHomesRecoverAfterLeadRestart: a node keeps its job running and reports after the lead returns.
func TestNodeTwoHomesRecoverAfterLeadRestart(t *testing.T) {
	t.Skip("pending: lead stopped during a job, node reports after restart, two homes, round 6")
}

// TestNodeCheckoutFailureFailsJobWithReason: a failed clone or worktree fails the job with a typed reason; nothing is marked started.
func TestNodeCheckoutFailureFailsJobWithReason(t *testing.T) {
	t.Skip("pending: checkout failure is failed with reason, no running report sent, round 5")
}

// TestNodeRefusesRepositoryOutsideAllowList: a job naming a repository not in NODE_REPOS is refused.
func TestNodeRefusesRepositoryOutsideAllowList(t *testing.T) {
	t.Skip("pending: repository not in NODE_REPOS refused before any work, round 5")
}

// TestCLIRunNeedSubmitsJob: agent run --need submits a job instead of starting a local task.
func TestCLIRunNeedSubmitsJob(t *testing.T) {
	t.Skip("pending: run --need and --on submit a job through the lead, round 7")
}

// TestDelegateRequiresCreatesJob: a delegate request with requires the local node lacks becomes a job on the lead.
func TestDelegateRequiresCreatesJob(t *testing.T) {
	t.Skip("pending: delegate with requires becomes a job based on the pushed SHA, round 8")
}

// TestDelegateResultReturnsHelperMessage: the finished job reaches the member as the ordinary helper-result message.
func TestDelegateResultReturnsHelperMessage(t *testing.T) {
	t.Skip("pending: delegated job result goes back as a helper-result message, round 8")
}

// TestWebJobsAndInstancesViews: the lead's web interface shows jobs and instances; the node marks job tasks.
func TestWebJobsAndInstancesViews(t *testing.T) {
	t.Skip("pending: Jobs and Instances views, job marker on a node task, round 9")
}
