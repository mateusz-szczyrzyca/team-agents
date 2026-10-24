package main

import (
	"testing"
)

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
