package main

import (
	"reflect"
	"testing"
)

// TestJobReportOmittedReplayInvalidStateCause distinguishes missing transition evidence from an
// unauthorized node or expired assignment, and checks that rejection preserves the accepted snapshot.
func TestJobReportOmittedReplayInvalidStateCause(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "omitted-cause", 1)
	first := JobReport{ID: "omitted-cause", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2), TaskID: "original"}
	if _, err := r.report(first, regAt(2)); err != nil {
		t.Fatalf("start: %v", err)
	}
	waiting := first
	waiting.State, waiting.At, waiting.TaskID = JobWaiting, regAt(3), "current"
	before, err := r.report(waiting, regAt(3))
	if err != nil {
		t.Fatalf("wait: %v", err)
	}
	first.At = Timestamp{}
	_, err = r.report(first, regAt(4))
	failsWith(t, LeadReportInvalidState)("timestamp-free resumption", err)
	after, gerr := r.get(first.ID)
	if gerr != nil {
		t.Fatalf("get after rejection: %v", gerr)
	}
	if !reflect.DeepEqual(before, after) {
		t.Fatalf("rejection changed accepted job: before=%+v after=%+v", before, after)
	}
}

// TestJobReportScalarPayloadInvalidStateCause checks each identifier independently so unrelated
// authorization failures cannot satisfy the report-payload conflict contract.
func TestJobReportScalarPayloadInvalidStateCause(t *testing.T) {
	for _, field := range []string{"branch", "task", "pr"} {
		t.Run(field, func(t *testing.T) {
			r := newLeadRegistry()
			assigned := reviewAssign(t, r, "payload-cause", 1)
			first := JobReport{ID: "payload-cause", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2), Branch: "branch-a", TaskID: "task-a", PRURL: "https://example.com/pr/1"}
			before, err := r.report(first, regAt(2))
			if err != nil {
				t.Fatalf("start: %v", err)
			}
			conflict := first
			conflict.At = regAt(4)
			switch field {
			case "branch":
				conflict.Branch = "branch-b"
			case "task":
				conflict.TaskID = "task-b"
			case "pr":
				conflict.PRURL = "https://example.com/pr/2"
			}
			_, err = r.report(conflict, regAt(4))
			failsWith(t, LeadReportInvalidState)("conflicting "+field, err)
			after, gerr := r.get(first.ID)
			if gerr != nil {
				t.Fatalf("get after rejection: %v", gerr)
			}
			if !reflect.DeepEqual(before, after) {
				t.Fatalf("rejection changed accepted job: before=%+v after=%+v", before, after)
			}
		})
	}
}
