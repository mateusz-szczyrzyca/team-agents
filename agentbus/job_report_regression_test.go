package main

import (
	"testing"
)

// TestLeadRegistryScalarReportStalePastTransitionDoesNotRegressState: a stale scalar report of a past
// transition that was already superseded by later transitions under the same assignment (even at the same
// timestamp) is recognized as a stale retry and does not regress the current state or mutate metadata.
func TestLeadRegistryScalarReportStalePastTransitionDoesNotRegressState(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}

	// First transition: running at t=2
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
		Branch:     "branch-1",
		At:         regAt(2),
	}, regAt(2)); err != nil {
		t.Fatalf("report running: %v", err)
	}

	// Second transition: waiting at t=2
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
		TaskID:     "task-1",
		Branch:     "branch-1",
		At:         regAt(2),
	}, regAt(2)); err != nil {
		t.Fatalf("report waiting: %v", err)
	}

	// Stale retry: the original running report at t=2 arrives again (e.g. network retransmission).
	// It must be accepted idempotently without regressing state back to running or mutating metadata.
	staleRep := JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "stale-task",
		Branch:     "stale-branch",
		At:         regAt(2),
	}
	res, err := r.report(staleRep, regAt(3))
	if err != nil {
		t.Fatalf("stale retry failed: %v", err)
	}
	if res.State != JobWaiting {
		t.Fatalf("stale retry regressed state to %s, want %s", res.State, JobWaiting)
	}
	if res.TaskID != "task-1" || res.Branch != "branch-1" {
		t.Fatalf("stale retry mutated metadata: task=%q branch=%q", res.TaskID, res.Branch)
	}

	job, errGet := r.get("job-1")
	if errGet != nil {
		t.Fatalf("get: %v", errGet)
	}
	if job.State != JobWaiting {
		t.Fatalf("stored job state = %s, want %s", job.State, JobWaiting)
	}
	if len(job.History) != 4 {
		t.Fatalf("stored history length = %d, want 4 (transitions: %+v)", len(job.History), job.History)
	}
}

// TestLeadRegistryScalarReportWaitingZeroAtDoesNotDuplicateHistory: retrying a waiting report with an omitted
// (zero) timestamp does not append another history entry.
func TestLeadRegistryScalarReportWaitingZeroAtDoesNotDuplicateHistory(t *testing.T) {
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
		At:         regAt(2),
	}, regAt(2)); err != nil {
		t.Fatalf("report running: %v", err)
	}

	// First waiting report with zero At (defaults to now=regAt(3))
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
	}, regAt(3)); err != nil {
		t.Fatalf("first waiting report: %v", err)
	}

	job, err := r.get("job-1")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if len(job.History) != 4 {
		t.Fatalf("history length after first waiting = %d, want 4", len(job.History))
	}

	// Retry identical waiting report with zero At at later time now=regAt(10)
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
	}, regAt(10)); err != nil {
		t.Fatalf("retry waiting report at t=10: %v", err)
	}

	job, err = r.get("job-1")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if len(job.History) != 4 {
		t.Fatalf("history length after retry at t=10 = %d, want 4 (duplicated waiting transition detected: %+v)", len(job.History), job.History)
	}

	// Retry again at now=regAt(20)
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobWaiting,
	}, regAt(20)); err != nil {
		t.Fatalf("retry waiting report at t=20: %v", err)
	}

	job, err = r.get("job-1")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if len(job.History) != 4 {
		t.Fatalf("history length after retry at t=20 = %d, want 4 (duplicated waiting transition detected: %+v)", len(job.History), job.History)
	}
}

// TestLeadRegistryTerminalCumulativeReportReplayWithDistinctReportAt: a cumulative terminal report with
// top-level At distinct from its final transition timestamp is accepted and accepts its own identical retry.
func TestLeadRegistryTerminalCumulativeReportReplayWithDistinctReportAt(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}

	rep := JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobDone,
		TaskID:     "task-1",
		Branch:     "branch-1",
		Result:     "success",
		At:         regAt(4), // Top-level report timestamp
		Transitions: []JobTransition{
			{State: JobRunning, At: regAt(2)},
			{State: JobDone, At: regAt(3)}, // Final transition timestamp is 3
		},
	}

	if _, err := r.report(rep, regAt(4)); err != nil {
		t.Fatalf("first cumulative terminal report: %v", err)
	}

	// Replay exact same cumulative report
	if _, err := r.report(rep, regAt(5)); err != nil {
		t.Fatalf("replay of cumulative terminal report rejected: %v", err)
	}
}

// TestLeadRegistryTerminalReportReplayWithEmptyResultAfterPriorResult: a terminal report with empty Result
// overwrites any prior non-terminal Result, and its own identical replay is accepted.
func TestLeadRegistryTerminalReportReplayWithEmptyResultAfterPriorResult(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-1"), regAt(0))
	got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if got == nil {
		t.Fatal("node-a did not get job-1")
	}

	// Non-terminal report with prior Result
	if _, err := r.report(JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
		Branch:     "branch-1",
		Result:     "intermediate status",
		At:         regAt(2),
	}, regAt(2)); err != nil {
		t.Fatalf("report running: %v", err)
	}

	// Terminal report with empty Result
	termRep := JobReport{
		ID:         "job-1",
		Node:       "node-a",
		Assignment: got.Assignment,
		State:      JobFailed,
		TaskID:     "task-1",
		Branch:     "branch-1",
		Result:     "",
		At:         regAt(3),
	}
	res, err := r.report(termRep, regAt(3))
	if err != nil {
		t.Fatalf("terminal report with empty result: %v", err)
	}
	if res.Result != "" {
		t.Fatalf("terminal report did not set empty Result, got %q", res.Result)
	}

	// Replay exact same terminal report with empty Result
	if _, err := r.report(termRep, regAt(4)); err != nil {
		t.Fatalf("replay of terminal report with empty result rejected: %v", err)
	}
}
