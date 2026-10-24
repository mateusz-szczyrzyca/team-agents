package main

import (
	"reflect"
	"sync"
	"testing"
)

// cancelActiveFixture seeds the accepted-start boundary without depending on the report implementation.
func cancelActiveFixture(t *testing.T, r *leadRegistry, state JobState) {
	t.Helper()
	regSubmit(t, r, regRequest("cancel-me"), regAt(0))
	assigned := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	if assigned == nil {
		t.Fatal("job not assigned")
	}
	job := r.jobs["cancel-me"]
	job.State = JobRunning
	job.History = append(job.History, JobTransition{State: JobRunning, At: regAt(2)})
	if state == JobWaiting {
		job.State = state
		job.History = append(job.History, JobTransition{State: state, At: regAt(3)})
	}
}

// TestJobCancelUnknownHasExactCause ensures missing jobs do not create cancellation records.
func TestJobCancelUnknownHasExactCause(t *testing.T) {
	r := newLeadRegistry()
	_, err := r.cancel("missing", regAt(0))
	failsWith(t, LeadCancelNoSuchJob)("unknown job", err)
	if len(r.jobs) != 0 {
		t.Fatal("missing cancellation created a job")
	}
}

// TestJobCancelTerminalIsNoop preserves a finished result when cancellation loses the completion race.
func TestJobCancelTerminalIsNoop(t *testing.T) {
	for _, state := range []JobState{JobDone, JobFailed, JobStopped} {
		t.Run(string(state), func(t *testing.T) {
			r := newLeadRegistry()
			original := regSubmit(t, r, regRequest("terminal"), regAt(0))
			original.State = state
			original.History = append(original.History, JobTransition{State: state, At: regAt(1)})
			r.jobs["terminal"] = &original
			got, err := r.cancel("terminal", regAt(2))
			if err != nil || !reflect.DeepEqual(got, original) {
				t.Fatalf("terminal cancellation revised outcome: %+v, %v", got, err)
			}
		})
	}
}

// TestJobCancelSnapshotBeforeStopFieldLoads preserves the first persisted registry format when adding cancellation.
func TestJobCancelSnapshotBeforeStopFieldLoads(t *testing.T) {
	// This immutable fixture is read without locking because no operation writes it.
	r, err := (&Bus{dir: "testdata/jobs-before-cancel"}).loadJobs()
	if err != nil {
		t.Fatal(err)
	}
	job := r.jobs["legacy"]
	if job == nil || job.StopRequested || job.State != JobQueued || r.nextSequence != 1 {
		t.Fatalf("old snapshot changed: %+v", job)
	}
}

// TestJobCancelReturnsOwnedSnapshot keeps callers from altering the stored cancellation history.
func TestJobCancelReturnsOwnedSnapshot(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("cancel-me"), regAt(0))
	stopped, err := r.cancel("cancel-me", regAt(1))
	if err != nil {
		t.Fatal(err)
	}
	if len(stopped.History) == 0 {
		t.Fatal("cancel returned empty history")
	}
	stopped.History[0].State = JobFailed
	if r.jobs["cancel-me"].History[0].State != JobQueued {
		t.Fatal("cancellation snapshot aliases stored history")
	}
}

// TestJobCancelConcurrentWithHeartbeat serializes cancellation with assignment and leaves no launchable work.
func TestJobCancelConcurrentWithHeartbeat(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("cancel-me"), regAt(0))
	var wg sync.WaitGroup
	for range 20 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := r.cancel("cancel-me", regAt(2)); err != nil {
				t.Errorf("cancel: %v", err)
			}
		}()
	}
	wg.Add(1)
	go func() {
		defer wg.Done()
		regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	}()
	wg.Wait()
	job := r.jobs["cancel-me"]
	if job.State != JobStopped || job.StopRequested || len(job.History) > 3 {
		t.Fatalf("concurrent cancel lost final state: %+v", job)
	}
	if got := regHeartbeat(t, r, regBeat("node-b", nil, FreeAgent{Name: "beta", Tier: TierNormal}), regAt(3)); got != nil {
		t.Fatalf("cancelled work reassigned: %+v", got)
	}
}
