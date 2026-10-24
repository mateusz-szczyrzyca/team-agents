package main

import (
	"fmt"
	"testing"
)

// TestJobCancelTerminalReportClearsPending preserves stop requests through waits and clears them on every terminal result.
func TestJobCancelTerminalReportClearsPending(t *testing.T) {
	for _, state := range []JobState{JobDone, JobFailed, JobStopped} {
		for _, cumulative := range []bool{false, true} {
			t.Run(fmt.Sprintf("%s/cumulative=%t", state, cumulative), func(t *testing.T) {
				b := newBus(t)
				err := transactJobs(b, LeadReportStorage, regAt(6), func(r *leadRegistry) *LeadReportError {
					regSubmit(t, r, regRequest("cancel-me"), regAt(0))
					assigned := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
					if assigned == nil {
						t.Fatal("job not assigned")
					}
					rep := JobReport{ID: "cancel-me", Node: "node-a", Assignment: assigned.Assignment, State: JobRunning, At: regAt(2), TaskID: "task-1", Branch: "branch-1"}
					if _, err := r.report(rep, regAt(2)); err != nil {
						return err
					}
					if _, err := r.cancel("cancel-me", regAt(3)); err != nil {
						t.Fatalf("cancel: %v", err)
					}
					for i, next := range []JobState{JobWaiting, JobRunning} {
						rep.State, rep.At = next, regAt(4+i)
						got, err := r.report(rep, rep.At)
						if err != nil {
							return err
						}
						if !got.StopRequested {
							t.Fatal("nonterminal report cleared pending stop")
						}
					}
					rep.State, rep.At, rep.Result = state, regAt(6), "final outcome"
					if cumulative {
						rep.Transitions = []JobTransition{{State: JobRunning, At: regAt(2)}, {State: JobWaiting, At: regAt(4)}, {State: JobRunning, At: regAt(5)}, {State: state, At: regAt(6)}}
					}
					got, err := r.report(rep, regAt(6))
					if err != nil {
						return err
					}
					if got.State != state || got.StopRequested || got.Result != "final outcome" {
						t.Fatalf("terminal report left pending stop or changed result: %+v", got)
					}
					return nil
				})
				if err != nil {
					t.Fatal(err)
				}
				storeRun(t, &Bus{dir: b.dir}, regAt(7), func(r *leadRegistry) {
					got, err := r.get("cancel-me")
					if err != nil || got.State != state || got.StopRequested || got.Result != "final outcome" {
						t.Fatalf("restart changed terminal outcome: %+v, %v", got, err)
					}
				})
			})
		}
	}
}

// TestJobCancelUnstartedRejectsLateStart refuses a launch acknowledgement after immediate cancellation.
func TestJobCancelUnstartedRejectsLateStart(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("cancel-me"), regAt(0))
	assigned := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if assigned == nil {
		t.Fatal("job not assigned")
	}
	if _, err := r.cancel("cancel-me", regAt(2)); err != nil {
		t.Fatal(err)
	}
	_, err := r.report(JobReport{ID: "cancel-me", Node: "node-a", Assignment: assigned.Assignment, State: JobRunning, At: regAt(3)}, regAt(3))
	failsWith(t, LeadReportTerminalConflict)("late start after cancellation", err)
}
