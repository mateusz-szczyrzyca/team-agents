package main

import (
	"reflect"
	"testing"
)

// TestJobReportCumulativeProgressMatchesOnlineDelivery verifies that valid progress reported cumulatively
// after an outage produces the exact same job state as separate online reports (investigator #29, M1).
func TestJobReportCumulativeProgressMatchesOnlineDelivery(t *testing.T) {
	// 1. Online delivery: separate reports for running@2, waiting@3, running@4 with updated result.
	rOnline := newLeadRegistry()
	assignedOnline := reviewAssign(t, rOnline, "job-online", 1)

	rep1Online := JobReport{
		ID:         "job-online",
		Node:       "node-a",
		Assignment: assignedOnline,
		State:      JobRunning,
		At:         regAt(2),
		Branch:     "branch-online",
		TaskID:     "task-online-1",
		Result:     "Result=A",
	}
	if _, err := rOnline.report(rep1Online, regAt(2)); err != nil {
		t.Fatalf("online rep1: %v", err)
	}

	rep2Online := JobReport{
		ID:         "job-online",
		Node:       "node-a",
		Assignment: assignedOnline,
		State:      JobWaiting,
		At:         regAt(3),
		Branch:     "branch-online",
		TaskID:     "task-online-1",
		Result:     "Result=A",
	}
	if _, err := rOnline.report(rep2Online, regAt(3)); err != nil {
		t.Fatalf("online rep2: %v", err)
	}

	rep3Online := JobReport{
		ID:         "job-online",
		Node:       "node-a",
		Assignment: assignedOnline,
		State:      JobRunning,
		At:         regAt(4),
		Branch:     "branch-online",
		TaskID:     "task-online-2",
		Result:     "Result=B",
	}
	if _, err := rOnline.report(rep3Online, regAt(4)); err != nil {
		t.Fatalf("online rep3: %v", err)
	}

	jobOnline, gerr := rOnline.get("job-online")
	if gerr != nil {
		t.Fatalf("get online: %v", gerr)
	}

	// 2. Cumulative delivery: initial running@2, then outage, then cumulative report with waiting@3, running@4.
	rCum := newLeadRegistry()
	assignedCum := reviewAssign(t, rCum, "job-cum", 1)

	rep1Cum := JobReport{
		ID:         "job-cum",
		Node:       "node-a",
		Assignment: assignedCum,
		State:      JobRunning,
		At:         regAt(2),
		Branch:     "branch-online",
		TaskID:     "task-online-1",
		Result:     "Result=A",
		Transitions: []JobTransition{
			{State: JobRunning, At: regAt(2)},
		},
	}
	if _, err := rCum.report(rep1Cum, regAt(2)); err != nil {
		t.Fatalf("cum rep1: %v", err)
	}

	rep2Cum := JobReport{
		ID:         "job-cum",
		Node:       "node-a",
		Assignment: assignedCum,
		State:      JobRunning,
		At:         regAt(4),
		Branch:     "branch-online",
		TaskID:     "task-online-2",
		Result:     "Result=B",
		Transitions: []JobTransition{
			{State: JobRunning, At: regAt(2)},
			{State: JobWaiting, At: regAt(3)},
			{State: JobRunning, At: regAt(4)},
		},
	}
	if _, err := rCum.report(rep2Cum, regAt(4)); err != nil {
		t.Fatalf("cum rep2 (cumulative progress): %v", err)
	}

	jobCum, gerr := rCum.get("job-cum")
	if gerr != nil {
		t.Fatalf("get cum: %v", gerr)
	}

	// Compare states and payloads
	if jobOnline.State != jobCum.State {
		t.Fatalf("state mismatch: online=%s cum=%s", jobOnline.State, jobCum.State)
	}
	if jobOnline.Result != jobCum.Result {
		t.Fatalf("result mismatch: online=%q cum=%q", jobOnline.Result, jobCum.Result)
	}
	if jobOnline.TaskID != jobCum.TaskID {
		t.Fatalf("taskID mismatch: online=%q cum=%q", jobOnline.TaskID, jobCum.TaskID)
	}
	if !reflect.DeepEqual(jobOnline.History, jobCum.History) {
		t.Fatalf("history mismatch: online=%+v cum=%+v", jobOnline.History, jobCum.History)
	}

	// 3. Replay with no new transitions cannot overwrite result
	sameState := rep2Cum
	sameState.Result = "Result=C"
	_, err := rCum.report(sameState, regAt(5))
	failsWith(t, LeadReportInvalidState)("conflicting result in same-state replay without new transitions", err)

	after, gerr := rCum.get("job-cum")
	if gerr != nil {
		t.Fatalf("get after rejection: %v", gerr)
	}
	if !reflect.DeepEqual(jobCum, after) {
		t.Fatalf("rejection changed accepted job: before=%+v after=%+v", jobCum, after)
	}
}
