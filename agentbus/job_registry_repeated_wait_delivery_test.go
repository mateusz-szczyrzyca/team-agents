package main

import (
	"reflect"
	"testing"
)

// TestJobReportRepeatedWaitDelivery verifies that reporting a new waiting transition
// with progress (new Result/TaskID) succeeds identically for both scalar and cumulative
// delivery, and that stale replays cannot overwrite newer data (investigator #30, M1).
func TestJobReportRepeatedWaitDelivery(t *testing.T) {
	// 1. Scalar delivery of repeated waiting with progress
	rScalar := newLeadRegistry()
	assignedScalar := reviewAssign(t, rScalar, "job-scalar-wait", 1)

	scalarRunning := JobReport{
		ID:         "job-scalar-wait",
		Node:       "node-a",
		Assignment: assignedScalar,
		State:      JobRunning,
		At:         regAt(2),
		Branch:     "branch-1",
		TaskID:     "task-1",
	}
	if _, err := rScalar.report(scalarRunning, regAt(2)); err != nil {
		t.Fatalf("scalar running: %v", err)
	}

	scalarWait1 := JobReport{
		ID:         "job-scalar-wait",
		Node:       "node-a",
		Assignment: assignedScalar,
		State:      JobWaiting,
		At:         regAt(3),
		Branch:     "branch-1",
		TaskID:     "task-1",
		Result:     "Result=A",
	}
	if _, err := rScalar.report(scalarWait1, regAt(3)); err != nil {
		t.Fatalf("scalar wait1: %v", err)
	}

	scalarWait2 := JobReport{
		ID:         "job-scalar-wait",
		Node:       "node-a",
		Assignment: assignedScalar,
		State:      JobWaiting,
		At:         regAt(4),
		Branch:     "branch-1",
		TaskID:     "task-1",
		Result:     "Result=B",
	}
	if _, err := rScalar.report(scalarWait2, regAt(4)); err != nil {
		t.Fatalf("scalar wait2 (new waiting transition with progress): %v", err)
	}

	jobScalar, err := rScalar.get("job-scalar-wait")
	if err != nil {
		t.Fatalf("get scalar: %v", err)
	}

	// 2. Cumulative delivery of equivalent repeated waiting with progress
	rCum := newLeadRegistry()
	assignedCum := reviewAssign(t, rCum, "job-cum-wait", 1)

	cumRunning := JobReport{
		ID:          "job-cum-wait",
		Node:        "node-a",
		Assignment:  assignedCum,
		State:       JobRunning,
		At:          regAt(2),
		Branch:      "branch-1",
		TaskID:      "task-1",
		Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
	}
	if _, err := rCum.report(cumRunning, regAt(2)); err != nil {
		t.Fatalf("cum running: %v", err)
	}

	cumWait1 := JobReport{
		ID:         "job-cum-wait",
		Node:       "node-a",
		Assignment: assignedCum,
		State:      JobWaiting,
		At:         regAt(3),
		Branch:     "branch-1",
		TaskID:     "task-1",
		Result:     "Result=A",
		Transitions: []JobTransition{
			{State: JobRunning, At: regAt(2)},
			{State: JobWaiting, At: regAt(3)},
		},
	}
	if _, err := rCum.report(cumWait1, regAt(3)); err != nil {
		t.Fatalf("cum wait1: %v", err)
	}

	cumWait2 := JobReport{
		ID:         "job-cum-wait",
		Node:       "node-a",
		Assignment: assignedCum,
		State:      JobWaiting,
		At:         regAt(4),
		Branch:     "branch-1",
		TaskID:     "task-1",
		Result:     "Result=B",
		Transitions: []JobTransition{
			{State: JobRunning, At: regAt(2)},
			{State: JobWaiting, At: regAt(3)},
			{State: JobWaiting, At: regAt(4)},
		},
	}
	if _, err := rCum.report(cumWait2, regAt(4)); err != nil {
		t.Fatalf("cum wait2: %v", err)
	}

	jobCum, err := rCum.get("job-cum-wait")
	if err != nil {
		t.Fatalf("get cum: %v", err)
	}

	// 3. Verify parity between scalar and cumulative delivery
	if jobScalar.State != JobWaiting || jobCum.State != JobWaiting {
		t.Fatalf("states: scalar=%s, cum=%s, want %s", jobScalar.State, jobCum.State, JobWaiting)
	}
	if jobScalar.Result != "Result=B" || jobCum.Result != "Result=B" {
		t.Fatalf("results: scalar=%q, cum=%q, want Result=B", jobScalar.Result, jobCum.Result)
	}
	if !reflect.DeepEqual(jobScalar.History, jobCum.History) {
		t.Fatalf("history mismatch:\nscalar: %+v\ncum:    %+v", jobScalar.History, jobCum.History)
	}

	// 4. Verify stale replays cannot overwrite newer data in scalar registry
	// a) Replay of wait1 with earlier timestamp (t=3 < t=4)
	_, errStaleTime := rScalar.report(scalarWait1, regAt(5))
	failsWith(t, LeadReportInvalidState)("scalar replay with earlier timestamp", errStaleTime)

	// b) Retry at current timestamp (t=4) with conflicting older Result=A
	scalarSameAtOlderResult := scalarWait2
	scalarSameAtOlderResult.Result = "Result=A"
	_, errSameAtOlder := rScalar.report(scalarSameAtOlderResult, regAt(5))
	failsWith(t, LeadReportInvalidState)("scalar same-at retry with older result", errSameAtOlder)

	// c) Stale retry with zero At timestamp
	scalarZeroAtOlderResult := scalarWait2
	scalarZeroAtOlderResult.At = Timestamp{}
	scalarZeroAtOlderResult.Result = "Result=A"
	_, errZeroAtOlder := rScalar.report(scalarZeroAtOlderResult, regAt(5))
	failsWith(t, LeadReportInvalidState)("scalar zero-at retry with older result", errZeroAtOlder)

	// Verify scalar snapshot untouched
	preservedScalar, err := rScalar.get("job-scalar-wait")
	if err != nil {
		t.Fatalf("get scalar preserved: %v", err)
	}
	if preservedScalar.Result != "Result=B" || len(preservedScalar.History) != 5 {
		t.Fatalf("scalar snapshot mutated: Result=%q, history len=%d", preservedScalar.Result, len(preservedScalar.History))
	}

	// 5. Verify stale replays cannot overwrite newer data in cumulative registry
	// a) Cumulative report regressing history length
	_, errCumRegress := rCum.report(cumWait1, regAt(5))
	failsWith(t, LeadReportInvalidState)("cum replay with regressed transitions", errCumRegress)

	// b) Cumulative report with identical history but conflicting older Result=A
	cumSameHistOlderResult := cumWait2
	cumSameHistOlderResult.Result = "Result=A"
	_, errCumOlderResult := rCum.report(cumSameHistOlderResult, regAt(5))
	failsWith(t, LeadReportInvalidState)("cum same-history retry with older result", errCumOlderResult)

	// Verify cumulative snapshot untouched
	preservedCum, err := rCum.get("job-cum-wait")
	if err != nil {
		t.Fatalf("get cum preserved: %v", err)
	}
	if preservedCum.Result != "Result=B" || len(preservedCum.History) != 5 {
		t.Fatalf("cum snapshot mutated: Result=%q, history len=%d", preservedCum.Result, len(preservedCum.History))
	}
}
