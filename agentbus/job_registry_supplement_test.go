package main

import (
	"encoding/json"
	"errors"
	"testing"
)

// TestRegistryCapabilitySetIsCanonical: the order and repetition of declared capabilities do not change a submission.
func TestRegistryCapabilitySetIsCanonical(t *testing.T) {
	r := newLeadRegistry()
	a := regRequest("job-1")
	a.Requires = []string{"os:linux", "hw:rp2350"}
	first := regSubmit(t, r, a, regAt(0))

	b := regRequest("job-1")
	b.Requires = []string{"hw:rp2350", "os:linux", "os:linux"}
	again, err := r.submit(b, regAt(1))
	if err != nil {
		t.Fatalf("same capability set in another order refused: %v", err)
	}
	if first.State != JobQueued || again.Request.ID != "job-1" {
		t.Fatalf("submission = %+v, want the queued job-1", first)
	}
	if again.Seq != first.Seq {
		t.Fatalf("resubmission Seq = %d, want the stored job %d", again.Seq, first.Seq)
	}
}

// TestRegistryHeartbeatRejectsInvalidFreeAgent: a free agent needs a name and a known tier.
func TestRegistryHeartbeatRejectsInvalidFreeAgent(t *testing.T) {
	cases := []struct {
		name  string
		agent FreeAgent
	}{
		{"empty agent name", FreeAgent{Name: "", Tier: TierNormal}},
		{"unknown tier", FreeAgent{Name: "alpha", Tier: Tier("bogus")}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			_, err := newLeadRegistry().heartbeat(regBeat("node-a", nil, c.agent), regAt(0))
			failsWith(t, LeadHeartbeatMalformedBody)(c.name, err)
		})
	}
}

// TestRegistryRefusedHeartbeatKeepsPreviousSnapshot: a refused heartbeat changes nothing; the node's last accepted
// declaration still decides routing and waiting reasons.
func TestRegistryRefusedHeartbeatKeepsPreviousSnapshot(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-hw")
	req.Requires = []string{"hw:rp2350"}
	regSubmit(t, r, req, regAt(0))

	if got := regHeartbeat(t, r, regBeat("node-a", []string{"os:linux", "hw:rp2350"}), regAt(1)); got != nil {
		t.Fatalf("node-a without a free agent got %+v", got)
	}
	_, err := r.heartbeat(regBeat("node-a", []string{"bogus"}, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(2))
	failsWith(t, LeadHeartbeatInvalidCapabilities)("refused capabilities", err)
	_, err = r.heartbeat(regBeat("node-a", []string{"os:linux", "hw:rp2350"}, FreeAgent{Name: "", Tier: TierNormal}), regAt(3))
	failsWith(t, LeadHeartbeatMalformedBody)("refused free agent", err)

	if reason := r.waitReason("job-hw", regAt(4), regTTL); reason != "no free normal agent" {
		t.Fatalf("reason = %q, want the last accepted snapshot (hw, no free normal agent)", reason)
	}
}

// TestRegistryHeartbeatCopiesSnapshot: the registry keeps its own copy of the capabilities and free agents
// that a node sent; later changes by the caller do not alter routing.
func TestRegistryHeartbeatCopiesSnapshot(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-hw")
	req.Requires = []string{"hw:rp2350"}
	regSubmit(t, r, req, regAt(0))

	caps := []string{"os:linux", "hw:rp2350"}
	free := []FreeAgent{{Name: "gamma", Tier: TierWorker}}
	if got := regHeartbeat(t, r, NodeHeartbeat{Node: "node-a", Capabilities: caps, FreeAgents: free}, regAt(1)); got != nil {
		t.Fatalf("node-a with only a worker free got %+v for a normal job", got)
	}
	caps[1] = "hw:other"
	free[0].Tier = TierNormal

	if reason := r.waitReason("job-hw", regAt(2), regTTL); reason != "no free normal agent" {
		t.Fatalf("reason = %q, want the copied snapshot (hw, worker free) to stand", reason)
	}
}

// TestRegistryHistoryReturnedCopyIsIsolated: changing the history of a returned job does not change the registry.
func TestRegistryHistoryReturnedCopyIsIsolated(t *testing.T) {
	r := newLeadRegistry()
	job := regSubmit(t, r, regRequest("job-1"), regAt(0))
	if len(job.History) != 1 {
		t.Fatalf("history after submit = %+v, want one entry", job.History)
	}
	job.History[0].State = JobDone

	again := regSubmit(t, r, regRequest("job-1"), regAt(1))
	if len(again.History) != 1 || again.History[0].State != JobQueued {
		t.Fatalf("stored history changed by the caller: %+v", again.History)
	}

	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(2))
	if got == nil || len(got.History) != 2 {
		t.Fatalf("assigned job = %+v, want two history entries", got)
	}
	got.History[1].State = JobDone
	retry := regHeartbeat(t, r, regBeat("node-a", nil), regAt(3))
	if retry == nil || len(retry.History) != 2 || retry.History[1].State != JobAssigned {
		t.Fatalf("retried assignment history = %+v, want the stored assigned entry", retry)
	}
}

// TestJobStateValuesRoundTrip: every defined state is valid and survives JSON; any other value is a *ValueError.
func TestJobStateValuesRoundTrip(t *testing.T) {
	states := []JobState{JobQueued, JobAssigned, JobRunning, JobWaiting, JobDone, JobFailed, JobStopped}
	for _, s := range states {
		if !s.Valid() {
			t.Fatalf("%q is not valid", s)
		}
		data, err := json.Marshal(s)
		if err != nil {
			t.Fatalf("marshal %q: %v", s, err)
		}
		var back JobState
		if err := json.Unmarshal(data, &back); err != nil || back != s {
			t.Fatalf("round trip of %q gave %q, err %v", s, back, err)
		}
	}

	for _, bad := range []string{"", "Queued", "cancelled", "assigned "} {
		if JobState(bad).Valid() {
			t.Fatalf("%q is valid", bad)
		}
		var s JobState
		err := json.Unmarshal([]byte(`"`+bad+`"`), &s)
		var verr *ValueError
		if !errors.As(err, &verr) {
			t.Fatalf("unmarshal %q: error %v is not a *ValueError", bad, err)
		}
	}

	var s JobState
	err := json.Unmarshal([]byte(`7`), &s)
	var verr *ValueError
	if !errors.As(err, &verr) {
		t.Fatalf("non-string state: error %v is not a *ValueError", err)
	}
}

// TestRegistryOutstandingAssignmentBlocksNode: a node with an unstarted assignment cannot take another job,
// even when its last heartbeat listed free agents; the wait reason says so.
func TestRegistryOutstandingAssignmentBlocksNode(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	regSubmit(t, r, regRequest("job-2"), regAt(0))
	free := []FreeAgent{{Name: "alpha", Tier: TierNormal}, {Name: "beta", Tier: TierNormal}}

	first := regHeartbeat(t, r, NodeHeartbeat{Node: "node-a", FreeAgents: free}, regAt(1))
	if first == nil || first.Request.ID != "job-1" {
		t.Fatalf("node-a got %+v, want job-1", first)
	}
	if reason := r.waitReason("job-2", regAt(2), regTTL); reason != "no free normal agent" {
		t.Fatalf("reason = %q, want the outstanding assignment to block node-a", reason)
	}
	again := regHeartbeat(t, r, NodeHeartbeat{Node: "node-a", FreeAgents: free}, regAt(3))
	if again == nil || again.Request.ID != "job-1" || again.Assignment != first.Assignment {
		t.Fatalf("node-a's next heartbeat = %+v, want job-1 again under assignment %d", again, first.Assignment)
	}
}

// TestRegistryWaitReasonNeedsOneNodeWithAllCapabilities: every capability is declared by some live node, but no
// single node has them all, so the reason names the whole set.
func TestRegistryWaitReasonNeedsOneNodeWithAllCapabilities(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-1")
	req.Requires = []string{"os:linux", "hw:rp2350"}
	regSubmit(t, r, req, regAt(0))
	alpha := FreeAgent{Name: "alpha", Tier: TierNormal}

	if got := regHeartbeat(t, r, regBeat("node-a", []string{"hw:rp2350"}, alpha), regAt(1)); got != nil {
		t.Fatalf("node-a with hw only got %+v", got)
	}
	if got := regHeartbeat(t, r, regBeat("node-b", []string{"os:linux"}, alpha), regAt(2)); got != nil {
		t.Fatalf("node-b with os only got %+v", got)
	}
	if reason := r.waitReason("job-1", regAt(3), regTTL); reason != "no node with os:linux and hw:rp2350" {
		t.Fatalf("reason = %q, want one node that has both capabilities", reason)
	}
}
