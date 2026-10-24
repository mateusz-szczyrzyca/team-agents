package main

import (
	"testing"
)

// TestJobReportCumulativeProgressMayUpdateTaskID verifies that a cumulative report which appends transitions
// and ends in the state already recorded is newer by its history, so it may update the task ID (investigator
// #29, M1). A same-state report without new transitions still cannot replace it.
func TestJobReportCumulativeProgressMayUpdateTaskID(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "cum-progress", 1)
	first := JobReport{
		ID:          "cum-progress",
		Node:        "node-a",
		Assignment:  assigned,
		State:       JobRunning,
		At:          regAt(2),
		TaskID:      "task-a",
		Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
	}
	if _, err := r.report(first, regAt(2)); err != nil {
		t.Fatalf("first cumulative report: %v", err)
	}

	progress := JobReport{
		ID:         "cum-progress",
		Node:       "node-a",
		Assignment: assigned,
		State:      JobRunning,
		At:         regAt(4),
		TaskID:     "task-b",
		Transitions: []JobTransition{
			{State: JobRunning, At: regAt(2)},
			{State: JobWaiting, At: regAt(3)},
			{State: JobRunning, At: regAt(4)},
		},
	}
	if _, err := r.report(progress, regAt(5)); err != nil {
		t.Fatalf("cumulative progress with new transitions: %v", err)
	}
	job, gerr := r.get("cum-progress")
	if gerr != nil {
		t.Fatalf("get: %v", gerr)
	}
	if job.TaskID != "task-b" {
		t.Fatalf("task ID after progress = %q, want task-b", job.TaskID)
	}
	if got := len(job.History); got != 5 {
		t.Fatalf("history length = %d, want 5 (assigned, running, running, waiting, running)", got)
	}

	sameState := JobReport{
		ID:          "cum-progress",
		Node:        "node-a",
		Assignment:  assigned,
		State:       JobRunning,
		At:          regAt(4),
		TaskID:      "task-a",
		Transitions: progress.Transitions,
	}
	_, rerr := r.report(sameState, regAt(6))
	failsWith(t, LeadReportInvalidState)("older task ID replayed with the same history", rerr)
}
