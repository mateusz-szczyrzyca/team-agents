package main

import "testing"

// reviewAssign submits job id and assigns it to node-a at time at; it returns the assignment evidence.
func reviewAssign(t *testing.T, r *leadRegistry, id string, at int) uint64 {
	t.Helper()
	regSubmit(t, r, regRequest(id), regAt(0))
	assigned := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(at))
	if assigned == nil {
		t.Fatal("job not assigned")
	}
	return assigned.Assignment
}

// reviewHistoryMonotonic fails the test when a job's history ever decreases, which the store refuses to load.
func reviewHistoryMonotonic(t *testing.T, job Job) {
	t.Helper()
	for i := 1; i < len(job.History); i++ {
		if job.History[i].At.Before(job.History[i-1].At.Time) {
			t.Fatalf("history decreases at index %d: %v after %v", i, job.History[i].At, job.History[i-1].At)
		}
	}
}

// TestJobCancelEarlierClockKeepsHistoryReadable: a cancellation stamped before the last transition (a clock
// moving backwards, or a time captured before waiting for the lock) must not write a history the store rejects.
func TestJobCancelEarlierClockKeepsHistoryReadable(t *testing.T) {
	b := newBus(t)
	err := transactJobs(b, LeadCancelStorage, regAt(10), func(r *leadRegistry) *LeadCancelError {
		reviewAssign(t, r, "cancel-early", 10)
		_, cerr := r.cancel("cancel-early", regAt(9))
		return cerr
	})
	if err != nil {
		t.Fatalf("cancel with an earlier clock: %v", err)
	}
	err = transactJobs(b, LeadCancelStorage, regAt(11), func(r *leadRegistry) *LeadCancelError {
		job, gerr := r.get("cancel-early")
		if gerr != nil {
			t.Fatalf("get: %v", gerr)
		}
		if job.State != JobStopped {
			t.Fatalf("state = %s, want stopped", job.State)
		}
		reviewHistoryMonotonic(t, job)
		return nil
	})
	if err != nil {
		t.Fatalf("reload after cancellation: %v", err)
	}
}

// TestJobReportEqualTimeStateChangeNeedsCumulativeHistory: a scalar report that changes state at the instant of
// the last transition cannot be ordered, so it is refused; the identical retry of the accepted report is a replay.
func TestJobReportEqualTimeStateChangeNeedsCumulativeHistory(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "equal-time", 1)
	rep := JobReport{ID: "equal-time", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2)}
	if _, err := r.report(rep, regAt(2)); err != nil {
		t.Fatalf("running: %v", err)
	}
	rep.State = JobWaiting
	_, err := r.report(rep, regAt(2))
	failsWith(t, LeadReportInvalidState)("waiting at the instant of running", err)
	job, gerr := r.get("equal-time")
	if gerr != nil {
		t.Fatalf("get: %v", gerr)
	}
	if job.State != JobRunning {
		t.Fatalf("refused report changed state to %s", job.State)
	}
	rep.State = JobRunning
	if _, err := r.report(rep, regAt(2)); err != nil {
		t.Fatalf("identical retry of running must replay, not resume: %v", err)
	}
}

// TestJobReportRepeatedWaitingIsNotANewTransition: a waiting report repeating the current state, sent again
// with no timestamp, does not append a second waiting entry.
func TestJobReportRepeatedWaitingIsNotANewTransition(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "repeat-waiting", 1)
	if _, err := r.report(JobReport{ID: "repeat-waiting", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2)}, regAt(2)); err != nil {
		t.Fatalf("running: %v", err)
	}
	waiting := JobReport{ID: "repeat-waiting", Node: "node-a", Assignment: assigned, State: JobWaiting, At: regAt(3)}
	if _, err := r.report(waiting, regAt(3)); err != nil {
		t.Fatalf("waiting: %v", err)
	}
	before, _ := r.get("repeat-waiting")
	waiting.At = Timestamp{}
	if _, err := r.report(waiting, regAt(5)); err != nil {
		t.Fatalf("repeated waiting: %v", err)
	}
	after, _ := r.get("repeat-waiting")
	if len(after.History) != len(before.History) {
		t.Fatalf("repeated waiting added %d transition(s)", len(after.History)-len(before.History))
	}
}

// TestJobReportCumulativeAtMustMatchLastTransition: the report's own time must equal its last cumulative
// transition, both on first acceptance and on the identical retry.
func TestJobReportCumulativeAtMustMatchLastTransition(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "cumulative-at", 1)
	rep := JobReport{ID: "cumulative-at", Node: "node-a", Assignment: assigned, State: JobDone, At: regAt(4), Result: "ok",
		Transitions: []JobTransition{{State: JobRunning, At: regAt(2)}, {State: JobDone, At: regAt(3)}}}
	_, err := r.report(rep, regAt(4))
	failsWith(t, LeadReportInvalidState)("cumulative report At differs from its last transition", err)
	rep.At = regAt(3)
	if _, err := r.report(rep, regAt(4)); err != nil {
		t.Fatalf("matching At: %v", err)
	}
	if _, err := r.report(rep, regAt(5)); err != nil {
		t.Fatalf("identical retry: %v", err)
	}
}

// TestJobReportTerminalResultIsTheTerminalPayload: a terminal report replaces an intermediate result, even with an
// empty one, and its identical retry is accepted.
func TestJobReportTerminalResultIsTheTerminalPayload(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "terminal-payload", 1)
	running := JobReport{ID: "terminal-payload", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2), Result: "intermediate output"}
	if _, err := r.report(running, regAt(2)); err != nil {
		t.Fatalf("running: %v", err)
	}
	done := JobReport{ID: "terminal-payload", Node: "node-a", Assignment: assigned, State: JobDone, At: regAt(3)}
	if _, err := r.report(done, regAt(3)); err != nil {
		t.Fatalf("done: %v", err)
	}
	if _, err := r.report(done, regAt(4)); err != nil {
		t.Fatalf("identical retry of done: %v", err)
	}
	job, gerr := r.get("terminal-payload")
	if gerr != nil {
		t.Fatalf("get: %v", gerr)
	}
	if job.Result != "" {
		t.Fatalf("result = %q, want the terminal report's empty result", job.Result)
	}
}

// TestJobReportOmittedTimeReplayCannotRewindProgress: a report without its time is not an identifiable
// transition, so a retry of an old running report after waiting must not move the job back or overwrite its
// task ID; the lead refuses it.
func TestJobReportOmittedTimeReplayCannotRewindProgress(t *testing.T) {
	r := newLeadRegistry()
	assigned := reviewAssign(t, r, "omitted-replay", 1)
	timed := JobReport{ID: "omitted-replay", Node: "node-a", Assignment: assigned, State: JobRunning, At: regAt(2), TaskID: "original"}
	if _, err := r.report(timed, regAt(2)); err != nil {
		t.Fatalf("running: %v", err)
	}
	waiting := JobReport{ID: "omitted-replay", Node: "node-a", Assignment: assigned, State: JobWaiting, At: regAt(3), TaskID: "new"}
	if _, err := r.report(waiting, regAt(3)); err != nil {
		t.Fatalf("waiting: %v", err)
	}
	replay := timed
	replay.At = Timestamp{}
	if _, err := r.report(replay, regAt(4)); err == nil {
		t.Fatal("replayed running report without its time was accepted")
	}
	job, err := r.get("omitted-replay")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if job.State != JobWaiting || job.TaskID != "new" {
		t.Fatalf("job rewound to %s with task %q", job.State, job.TaskID)
	}
}
