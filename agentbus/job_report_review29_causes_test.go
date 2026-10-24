package main

import (
	"reflect"
	"testing"
)

// TestJobReportReview29PayloadConflictCauses pins the precise rejection cause for scalar results and
// cumulative payload replays, and requires rejected reports to preserve the complete accepted snapshot.
func TestJobReportReview29PayloadConflictCauses(t *testing.T) {
	for _, form := range []string{"scalar", "cumulative"} {
		for _, field := range []string{"branch", "task", "pr", "result"} {
			t.Run(form+"/"+field, func(t *testing.T) {
				r := newLeadRegistry()
				assigned := reviewAssign(t, r, "review29-cause", 1)
				first := JobReport{ID: "review29-cause", Node: "node-a", Assignment: assigned,
					State: JobRunning, At: regAt(2), Branch: "branch-a", TaskID: "task-a",
					PRURL: "https://example.com/pr/1", Result: "result-a"}
				if form == "cumulative" {
					first.Transitions = []JobTransition{{State: JobRunning, At: regAt(2)}}
				}
				before, err := r.report(first, regAt(2))
				if err != nil {
					t.Fatalf("accept initial report: %v", err)
				}
				conflict := first
				if form == "scalar" {
					conflict.At = regAt(4)
				}
				switch field {
				case "branch":
					conflict.Branch = "branch-b"
				case "task":
					conflict.TaskID = "task-b"
				case "pr":
					conflict.PRURL = "https://example.com/pr/2"
				case "result":
					conflict.Result = "result-b"
				}
				_, err = r.report(conflict, regAt(4))
				failsWith(t, LeadReportInvalidState)("conflicting "+field, err)
				after, gerr := r.get(first.ID)
				if gerr != nil {
					t.Fatalf("get after rejection: %v", gerr)
				}
				if !reflect.DeepEqual(before, after) {
					t.Fatalf("rejection changed job: before=%+v after=%+v", before, after)
				}
				replayed, rerr := r.report(first, regAt(5))
				if rerr != nil {
					t.Fatalf("retry accepted report: %v", rerr)
				}
				if !reflect.DeepEqual(before, replayed) {
					t.Fatalf("retry changed job: before=%+v after=%+v", before, replayed)
				}
			})
		}
	}
}
