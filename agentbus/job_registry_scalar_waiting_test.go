package main

import "testing"

// TestJobReportScalarWaitingProgressIsNotAReplay: a second scalar waiting report with a later time appends a
// new transition, so it is valid progress and must be accepted, as the same progress sent cumulatively is.
// Investigator #30 M1: the scalar path appended the transition but left the replay guard armed.
func TestJobReportScalarWaitingProgressIsNotAReplay(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "scalar-waiting", 1)
	if _, err := r.report(JobReport{ID: "scalar-waiting", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2)}, regAt(2)); err != nil {
		t.Fatalf("running: %v", err)
	}
	first := JobReport{ID: "scalar-waiting", Node: "node-a", Assignment: assigned, State: JobWaiting, At: regAt(3), Result: "A"}
	if _, err := r.report(first, regAt(3)); err != nil {
		t.Fatalf("first waiting: %v", err)
	}
	before, err := r.get("scalar-waiting")
	if err != nil {
		t.Fatalf("get before: %v", err)
	}
	second := JobReport{ID: "scalar-waiting", Node: "node-a", Assignment: assigned, State: JobWaiting, At: regAt(4), Result: "B"}
	if _, err := r.report(second, regAt(4)); err != nil {
		t.Fatalf("second waiting with a later time must be accepted as progress: %v", err)
	}
	job, err := r.get("scalar-waiting")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if len(job.History) != len(before.History)+1 {
		t.Fatalf("history has %d entries, want %d (one new waiting transition)", len(job.History), len(before.History)+1)
	}
	reviewHistoryMonotonic(t, job)
}
