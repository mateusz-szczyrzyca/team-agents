package main

import (
	"testing"
)

// TestJobReportCumulativePayloadCannotBeOverwritten verifies that cumulative reports cannot overwrite
// already-recorded branch, task ID, PR URL, or non-terminal result (investigator #28, M2).
func TestJobReportCumulativePayloadCannotBeOverwritten(t *testing.T) {
	t.Run("branch", func(t *testing.T) {
		r := newLeadRegistry()
		assigned := reviewAssign(t, r, "cum-payload-branch", 1)
		first := JobReport{
			ID:          "cum-payload-branch",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			Branch:      "branch-a",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		if _, err := r.report(first, regAt(2)); err != nil {
			t.Fatalf("first cumulative report: %v", err)
		}

		conflicting := JobReport{
			ID:          "cum-payload-branch",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			Branch:      "branch-b",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		_, err := r.report(conflicting, regAt(3))
		failsWith(t, LeadReportInvalidState)("conflicting cumulative branch", err)

		if _, err := r.report(first, regAt(4)); err != nil {
			t.Fatalf("identical retry: %v", err)
		}
		job, gerr := r.get("cum-payload-branch")
		if gerr != nil {
			t.Fatalf("get: %v", gerr)
		}
		if job.Branch != "branch-a" {
			t.Fatalf("branch overwritten to %q", job.Branch)
		}
	})

	t.Run("task", func(t *testing.T) {
		r := newLeadRegistry()
		assigned := reviewAssign(t, r, "cum-payload-task", 1)
		first := JobReport{
			ID:          "cum-payload-task",
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

		conflicting := JobReport{
			ID:          "cum-payload-task",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			TaskID:      "task-b",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		_, err := r.report(conflicting, regAt(3))
		failsWith(t, LeadReportInvalidState)("conflicting cumulative task ID", err)

		if _, err := r.report(first, regAt(4)); err != nil {
			t.Fatalf("identical retry: %v", err)
		}
		job, gerr := r.get("cum-payload-task")
		if gerr != nil {
			t.Fatalf("get: %v", gerr)
		}
		if job.TaskID != "task-a" {
			t.Fatalf("task ID overwritten to %q", job.TaskID)
		}
	})

	t.Run("pr", func(t *testing.T) {
		r := newLeadRegistry()
		assigned := reviewAssign(t, r, "cum-payload-pr", 1)
		first := JobReport{
			ID:          "cum-payload-pr",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			PRURL:       "https://example.com/pr/1",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		if _, err := r.report(first, regAt(2)); err != nil {
			t.Fatalf("first cumulative report: %v", err)
		}

		conflicting := JobReport{
			ID:          "cum-payload-pr",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			PRURL:       "https://example.com/pr/2",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		_, err := r.report(conflicting, regAt(3))
		failsWith(t, LeadReportInvalidState)("conflicting cumulative PR URL", err)

		if _, err := r.report(first, regAt(4)); err != nil {
			t.Fatalf("identical retry: %v", err)
		}
		job, gerr := r.get("cum-payload-pr")
		if gerr != nil {
			t.Fatalf("get: %v", gerr)
		}
		if job.PRURL != "https://example.com/pr/1" {
			t.Fatalf("PR URL overwritten to %q", job.PRURL)
		}
	})

	t.Run("result", func(t *testing.T) {
		r := newLeadRegistry()
		assigned := reviewAssign(t, r, "cum-payload-result", 1)
		first := JobReport{
			ID:          "cum-payload-result",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			Result:      "res-a",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		if _, err := r.report(first, regAt(2)); err != nil {
			t.Fatalf("first cumulative report: %v", err)
		}

		conflicting := JobReport{
			ID:          "cum-payload-result",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			Result:      "res-b",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		_, err := r.report(conflicting, regAt(3))
		failsWith(t, LeadReportInvalidState)("conflicting cumulative non-terminal result", err)

		if _, err := r.report(first, regAt(4)); err != nil {
			t.Fatalf("identical retry: %v", err)
		}
		job, gerr := r.get("cum-payload-result")
		if gerr != nil {
			t.Fatalf("get: %v", gerr)
		}
		if job.Result != "res-a" {
			t.Fatalf("result overwritten to %q", job.Result)
		}
	})

	t.Run("multi_transition", func(t *testing.T) {
		r := newLeadRegistry()
		assigned := reviewAssign(t, r, "cum-payload-multi", 1)
		first := JobReport{
			ID:          "cum-payload-multi",
			Node:        "node-a",
			Assignment:  assigned,
			State:       JobRunning,
			At:          regAt(2),
			Branch:      "branch-a",
			Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}},
		}
		if _, err := r.report(first, regAt(2)); err != nil {
			t.Fatalf("first cumulative report: %v", err)
		}

		conflicting := JobReport{
			ID:         "cum-payload-multi",
			Node:       "node-a",
			Assignment: assigned,
			State:      JobWaiting,
			At:         regAt(4),
			Branch:     "branch-b",
			Transitions: []JobTransition{
				{State: JobRunning, At: regAt(2)},
				{State: JobWaiting, At: regAt(4)},
			},
		}
		_, err := r.report(conflicting, regAt(4))
		failsWith(t, LeadReportInvalidState)("conflicting cumulative branch in multi-transition", err)
	})
}
