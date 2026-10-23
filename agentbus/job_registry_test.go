package main

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

// regSHA is a well-formed exact commit used by valid test submissions.
const regSHA = "0123456789abcdef0123456789abcdef01234567"

// regTTL is the node liveness window used by the registry tests.
const regTTL = 60 * time.Second

// regAt returns the test instant n seconds after a fixed start.
func regAt(n int) Timestamp {
	return Timestamp{time.Date(2026, time.October, 23, 10, 0, 0, 0, time.UTC).Add(time.Duration(n) * time.Second)}
}

// regRequest returns a valid submission with the given ID. Tests change one field at a time.
func regRequest(id string) JobRequest {
	return JobRequest{
		ID:         id,
		Repository: "team-agents",
		BaseRef:    "main",
		BaseSHA:    regSHA,
		Selector:   string(TierNormal),
		Task:       "run the checks",
		Requester:  "human",
	}
}

// regBeat returns a heartbeat of node with the given capabilities and free agents.
func regBeat(node string, caps []string, free ...FreeAgent) NodeHeartbeat {
	return NodeHeartbeat{Node: node, Capabilities: caps, FreeAgents: free}
}

// regSubmit submits req and fails the test when the submission is refused.
func regSubmit(t *testing.T, r *leadRegistry, req JobRequest, now Timestamp) Job {
	t.Helper()
	job, err := r.submit(req, now)
	if err != nil {
		t.Fatalf("submit %s: %v", req.ID, err)
	}
	return job
}

// regHeartbeat sends h and fails the test when the heartbeat is refused. It returns the assigned job or nil.
func regHeartbeat(t *testing.T, r *leadRegistry, h NodeHeartbeat, now Timestamp) *Job {
	t.Helper()
	got, err := r.heartbeat(h, now)
	if err != nil {
		t.Fatalf("heartbeat %s: %v", h.Node, err)
	}
	return got
}

// TestRegistrySubmitRejectsInvalidFields: every invalid submitted field is refused with its own cause.
func TestRegistrySubmitRejectsInvalidFields(t *testing.T) {
	cases := []struct {
		name string
		edit func(*JobRequest)
		want LeadSubmitCause
	}{
		{"empty ID", func(q *JobRequest) { q.ID = "" }, LeadSubmitMissingArgument},
		{"empty repository", func(q *JobRequest) { q.Repository = "" }, LeadSubmitMissingArgument},
		{"empty selector", func(q *JobRequest) { q.Selector = "" }, LeadSubmitMissingArgument},
		{"empty task", func(q *JobRequest) { q.Task = "" }, LeadSubmitMissingArgument},
		{"empty requester", func(q *JobRequest) { q.Requester = "" }, LeadSubmitMissingArgument},
		{"ID starts with a dash", func(q *JobRequest) { q.ID = "-x" }, LeadSubmitInvalidID},
		{"ID starts with an underscore", func(q *JobRequest) { q.ID = "_x" }, LeadSubmitInvalidID},
		{"ID contains a slash", func(q *JobRequest) { q.ID = "a/b" }, LeadSubmitInvalidID},
		{"ID contains a space", func(q *JobRequest) { q.ID = "a b" }, LeadSubmitInvalidID},
		{"ID longer than 128", func(q *JobRequest) { q.ID = strings.Repeat("a", 129) }, LeadSubmitInvalidID},
		{"base ref starts with a dash", func(q *JobRequest) { q.BaseRef = "-main" }, LeadSubmitInvalidBase},
		{"base ref leaves the repository", func(q *JobRequest) { q.BaseRef = "../etc" }, LeadSubmitInvalidBase},
		{"base ref contains a space", func(q *JobRequest) { q.BaseRef = "main branch" }, LeadSubmitInvalidBase},
		{"base SHA is not hex", func(q *JobRequest) { q.BaseSHA = strings.Repeat("z", 40) }, LeadSubmitInvalidBase},
		{"base SHA has 39 digits", func(q *JobRequest) { q.BaseSHA = regSHA[:39] }, LeadSubmitInvalidBase},
		{"agent job without an exact SHA", func(q *JobRequest) { q.Requester = "agent@node-a"; q.BaseSHA = "" }, LeadSubmitInvalidBase},
		{"requester without instance", func(q *JobRequest) { q.Requester = "agent" }, LeadSubmitMalformedBody},
		{"requester with empty instance", func(q *JobRequest) { q.Requester = "agent@" }, LeadSubmitMalformedBody},
		{"requester with two separators", func(q *JobRequest) { q.Requester = "agent@node@x" }, LeadSubmitMalformedBody},
		{"requester with a space", func(q *JobRequest) { q.Requester = "agent node@x" }, LeadSubmitMalformedBody},
		{"capability without namespace", func(q *JobRequest) { q.Requires = []string{"hw"} }, LeadSubmitInvalidCapabilities},
		{"capability with empty value", func(q *JobRequest) { q.Requires = []string{"hw:"} }, LeadSubmitInvalidCapabilities},
		{"capability with unknown namespace", func(q *JobRequest) { q.Requires = []string{"cpu:x86"} }, LeadSubmitInvalidCapabilities},
		{"capability with upper case value", func(q *JobRequest) { q.Requires = []string{"os:Linux"} }, LeadSubmitInvalidCapabilities},
		{"empty capability", func(q *JobRequest) { q.Requires = []string{""} }, LeadSubmitInvalidCapabilities},
		{"task above 65536 bytes", func(q *JobRequest) { q.Task = strings.Repeat("x", 65537) }, LeadSubmitTaskTooLarge},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			req := regRequest("job-1")
			c.edit(&req)
			_, err := newLeadRegistry().submit(req, regAt(0))
			failsWith(t, c.want)(c.name, err)
		})
	}
}

// TestRegistrySubmitAcceptsBoundaryFields: values at the limits of the rules are accepted and queued.
func TestRegistrySubmitAcceptsBoundaryFields(t *testing.T) {
	cases := []struct {
		name string
		edit func(*JobRequest)
	}{
		{"ID with dots, dashes and underscores", func(q *JobRequest) { q.ID = "a.B_c-9" }},
		{"ID of exactly 128 characters", func(q *JobRequest) { q.ID = "a" + strings.Repeat("b", 127) }},
		{"ref with a slash and a dot", func(q *JobRequest) { q.BaseRef = "release/1.2" }},
		{"SHA-256 commit", func(q *JobRequest) { q.BaseSHA = strings.Repeat("ab", 32) }},
		{"upper case SHA", func(q *JobRequest) { q.BaseSHA = strings.ToUpper(regSHA) }},
		{"human job without an exact SHA", func(q *JobRequest) { q.BaseSHA = "" }},
		{"agent job with instance and exact SHA", func(q *JobRequest) { q.Requester = "agent-1@node.a_b" }},
		{"capabilities of every namespace", func(q *JobRequest) {
			q.Requires = []string{"os:linux", "arch:arm64", "build:go", "test:bats", "hw:rp2350", "net:tailnet"}
		}},
		{"task of exactly 65536 bytes", func(q *JobRequest) { q.Task = strings.Repeat("x", 65536) }},
		{"pinned to a node", func(q *JobRequest) { q.Target = "node-b" }},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			req := regRequest("job-1")
			c.edit(&req)
			job, err := newLeadRegistry().submit(req, regAt(0))
			if err != nil {
				t.Fatalf("submit refused: %v", err)
			}
			if job.State != JobQueued {
				t.Fatalf("state = %q, want %q", job.State, JobQueued)
			}
		})
	}
}

// TestRegistrySubmitConflictingFieldsRefused: an existing ID never takes a different submission, and the
// refused submission leaves the stored job unchanged.
func TestRegistrySubmitConflictingFieldsRefused(t *testing.T) {
	base := regRequest("job-1")
	base.Requires = []string{"os:linux"}
	base.Target = "node-a"
	r := newLeadRegistry()
	first := regSubmit(t, r, base, regAt(0))

	changes := []struct {
		name string
		edit func(*JobRequest)
	}{
		{"repository", func(q *JobRequest) { q.Repository = "other" }},
		{"base ref", func(q *JobRequest) { q.BaseRef = "develop" }},
		{"base SHA", func(q *JobRequest) { q.BaseSHA = strings.Repeat("cd", 20) }},
		{"requires", func(q *JobRequest) { q.Requires = []string{"os:linux", "hw:rp2350"} }},
		{"selector", func(q *JobRequest) { q.Selector = string(TierWorker) }},
		{"target", func(q *JobRequest) { q.Target = "node-b" }},
		{"task", func(q *JobRequest) { q.Task = "something else" }},
		{"requester", func(q *JobRequest) { q.Requester = "agent@node-b" }},
	}
	for _, c := range changes {
		req := regRequest("job-1")
		req.Requires = []string{"os:linux"}
		req.Target = "node-a"
		c.edit(&req)
		_, err := r.submit(req, regAt(1))
		failsWith(t, LeadSubmitJobConflict)(c.name, err)
	}

	again := regSubmit(t, r, base, regAt(2))
	if again.Seq != first.Seq || again.Request.Task != base.Task || again.Request.Target != "node-a" {
		t.Fatalf("stored job changed after refused submissions: %+v", again)
	}
}

// TestRegistrySubmitCopiesRequest: the registry keeps its own copy of the request and of the job it returns.
func TestRegistrySubmitCopiesRequest(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-1")
	req.Requires = []string{"os:linux"}
	job := regSubmit(t, r, req, regAt(0))

	req.Requires[0] = "hw:rp2350"
	if len(job.Request.Requires) != 1 {
		t.Fatalf("returned job has %d requirements, want 1", len(job.Request.Requires))
	}
	job.Request.Requires[0] = "arch:arm64"

	original := regRequest("job-1")
	original.Requires = []string{"os:linux"}
	again, err := r.submit(original, regAt(1))
	if err != nil {
		t.Fatalf("identical resubmission refused after the caller changed its slices: %v", err)
	}
	if len(again.Request.Requires) != 1 || again.Request.Requires[0] != "os:linux" {
		t.Fatalf("stored requirements = %v, want [os:linux]", again.Request.Requires)
	}
}

// TestRegistryHeartbeatRejectsInvalidInput: a heartbeat with a missing node name or invalid capabilities is refused.
func TestRegistryHeartbeatRejectsInvalidInput(t *testing.T) {
	cases := []struct {
		name string
		h    NodeHeartbeat
		want LeadHeartbeatCause
	}{
		{"empty node name", regBeat("", nil), LeadHeartbeatMalformedBody},
		{"unknown namespace", regBeat("node-a", []string{"bogus"}), LeadHeartbeatInvalidCapabilities},
		{"empty capability value", regBeat("node-a", []string{"hw:"}), LeadHeartbeatInvalidCapabilities},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			_, err := newLeadRegistry().heartbeat(c.h, regAt(0))
			failsWith(t, c.want)(c.name, err)
		})
	}
}

// TestRegistryRetryReturnsSameAssignment: a node that lost the reply of its assignment gets the same assignment
// back, even when its capacity changed. The retry never takes a second job, and the next node gets the next job.
func TestRegistryRetryReturnsSameAssignment(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	regSubmit(t, r, regRequest("job-2"), regAt(0))
	free := FreeAgent{Name: "alpha", Tier: TierNormal}

	first := regHeartbeat(t, r, regBeat("node-a", nil, free), regAt(1))
	if first == nil || first.Request.ID != "job-1" || first.Assignment == 0 {
		t.Fatalf("first heartbeat = %+v, want job-1 with an assignment", first)
	}

	again := regHeartbeat(t, r, regBeat("node-a", nil), regAt(2))
	if again == nil || again.Request.ID != "job-1" || again.Assignment != first.Assignment {
		t.Fatalf("retry = %+v, want job-1 under assignment %d", again, first.Assignment)
	}

	other := regHeartbeat(t, r, regBeat("node-b", nil, free), regAt(3))
	if other == nil || other.Request.ID != "job-2" || other.Assignment <= first.Assignment {
		t.Fatalf("node-b got %+v, want job-2 under a newer assignment", other)
	}
}

// TestJobStateRejectsUnknownValue: an unknown job state is refused with a *ValueError, as tiers are.
func TestJobStateRejectsUnknownValue(t *testing.T) {
	var s JobState
	if err := json.Unmarshal([]byte(`"bogus"`), &s); err == nil {
		t.Fatal("unknown job state accepted")
	} else {
		var verr *ValueError
		if !errors.As(err, &verr) {
			t.Fatalf("error %v is not a *ValueError", err)
		}
	}
	if err := json.Unmarshal([]byte(`"assigned"`), &s); err != nil || s != JobAssigned {
		t.Fatalf("assigned state: got %q, err %v", s, err)
	}
}

// TestRegistryHistoryRecordsQueuedAndAssigned: the history keeps the queued and assigned transitions with their
// times, and neither a resubmission nor a retried heartbeat adds an entry.
func TestRegistryHistoryRecordsQueuedAndAssigned(t *testing.T) {
	r := newLeadRegistry()
	queued := regSubmit(t, r, regRequest("job-1"), regAt(0))
	if len(queued.History) != 1 || queued.History[0].State != JobQueued || !queued.History[0].At.Equal(regAt(0).Time) {
		t.Fatalf("history after submit = %+v, want [queued at submit time]", queued.History)
	}
	regSubmit(t, r, regRequest("job-1"), regAt(5))

	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(6))
	if got == nil || len(got.History) != 2 {
		t.Fatalf("assigned job = %+v, want two history entries", got)
	}
	if got.History[0].State != JobQueued || got.History[1].State != JobAssigned || !got.History[1].At.Equal(regAt(6).Time) {
		t.Fatalf("history = %+v, want queued then assigned at the heartbeat time", got.History)
	}

	retry := regHeartbeat(t, r, regBeat("node-a", nil), regAt(7))
	if retry == nil || len(retry.History) != 2 {
		t.Fatalf("retried assignment = %+v, want the same two entries", retry)
	}
}
