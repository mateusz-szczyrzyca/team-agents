package main

import (
	"os"
	"reflect"
	"strconv"
	"sync"
	"testing"
)

// storeRun executes a successful job-store transaction; every call reopens disk state.
func storeRun(t *testing.T, b *Bus, now Timestamp, fn func(*leadRegistry)) {
	t.Helper()
	called := false
	if err := transactJobs(b, LeadSubmitStorage, now, func(r *leadRegistry) *LeadSubmitError { called = true; fn(r); return nil }); err != nil {
		t.Fatalf("job transaction: %v", err)
	}
	if !called {
		t.Fatal("registry operation was not invoked")
	}
}

// TestJobStoreRecoversAssignmentAndSequence verifies that restart cannot reuse assignment evidence or reorder jobs.
func TestJobStoreRecoversAssignmentAndSequence(t *testing.T) {
	b := newBus(t)
	var first *Job
	storeRun(t, b, regAt(0), func(r *leadRegistry) {
		regSubmit(t, r, regRequest("first"), regAt(0))
		regSubmit(t, r, regRequest("second"), regAt(0))
		first = regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
	})
	if first == nil {
		t.Fatal("store did not assign first job")
	}
	storeRun(t, &Bus{dir: b.dir}, regAt(2), func(r *leadRegistry) {
		again := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(2))
		if !reflect.DeepEqual(first, again) {
			t.Fatalf("retry changed assignment: %+v, want %+v", again, first)
		}
		second := regHeartbeat(t, r, regBeat("node-b", nil, FreeAgent{Name: "beta", Tier: TierNormal}), regAt(2))
		if second == nil || second.Request.ID != "second" || second.Assignment <= first.Assignment {
			t.Fatalf("assignment identity/order lost: %+v", second)
		}
		third := regSubmit(t, r, regRequest("third"), regAt(2))
		if third.Seq <= second.Seq || r.nodes["node-a"].LastSeen != regAt(2) {
			t.Fatal("sequence or node registry lost")
		}
	})
}

// TestJobStoreFailedOperationDoesNotCommit keeps rejected mutations out of the durable snapshot.
func TestJobStoreFailedOperationDoesNotCommit(t *testing.T) {
	b := newBus(t)
	storeRun(t, b, regAt(0), func(r *leadRegistry) { regSubmit(t, r, regRequest("first"), regAt(0)) })
	err := transactJobs(b, LeadSubmitStorage, regAt(1), func(r *leadRegistry) *LeadSubmitError {
		regSubmit(t, r, regRequest("discarded"), regAt(1))
		return opErr(LeadSubmitJobConflict, "refused")
	})
	failsWith(t, LeadSubmitJobConflict)("rejected transaction", err)
	storeRun(t, b, regAt(2), func(r *leadRegistry) {
		if len(r.jobs) != 1 || r.jobs["first"] == nil {
			t.Fatalf("rejected operation persisted: %+v", r.jobs)
		}
	})
}

// TestJobStoreConcurrentHandles verifies distinct Bus handles serialize read-modify-write through flock.
func TestJobStoreConcurrentHandles(t *testing.T) {
	b := newBus(t)
	var wg sync.WaitGroup
	for i := range 12 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			storeRun(t, &Bus{dir: b.dir}, regAt(i), func(r *leadRegistry) { regSubmit(t, r, regRequest(strconv.Itoa(i)), regAt(i)) })
		}()
	}
	wg.Wait()
	storeRun(t, b, regAt(15), func(r *leadRegistry) {
		if len(r.jobs) != 12 || r.nextSequence != 12 {
			t.Fatalf("concurrent writes lost: %d jobs, sequence %d", len(r.jobs), r.nextSequence)
		}
	})
}

// TestJobStoreWriteFailureHasOperationCause checks failed replacement keeps the original file intact.
func TestJobStoreWriteFailureHasOperationCause(t *testing.T) {
	b := newBus(t)
	if err := os.Mkdir(b.path("jobs.json.tmp"), 0o700); err != nil {
		t.Fatal(err)
	}
	err := transactJobs(b, LeadHeartbeatStorage, regAt(0), func(*leadRegistry) *LeadHeartbeatError { return nil })
	failsWith(t, LeadHeartbeatStorage)("cannot write snapshot", err)
}

// TestJobStoreCumulativeHistoryRoundtrip verifies that multi-step cumulative transition history and
// metadata persist across disk reloads.
func TestJobStoreCumulativeHistoryRoundtrip(t *testing.T) {
	b := newBus(t)
	var assignment uint64
	storeRun(t, b, regAt(0), func(r *leadRegistry) {
		regSubmit(t, r, regRequest("job-1"), regAt(0))
		got := regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
		if got == nil {
			t.Fatal("heartbeat did not assign job-1")
		}
		assignment = got.Assignment
	})

	// Report cumulative transitions: running -> waiting -> running
	transitions := []JobTransition{
		{State: JobRunning, At: regAt(2)},
		{State: JobWaiting, At: regAt(3)},
		{State: JobRunning, At: regAt(4)},
	}
	storeRun(t, b, regAt(4), func(r *leadRegistry) {
		_, err := r.report(JobReport{
			ID:          "job-1",
			Node:        "node-a",
			Assignment:  assignment,
			State:       JobRunning,
			TaskID:      "task-1",
			Branch:      "branch-1",
			Transitions: transitions,
		}, regAt(4))
		if err != nil {
			t.Fatalf("report cumulative transitions: %v", err)
		}
	})

	// Reopen from disk and verify cumulative history and fields survived
	storeRun(t, &Bus{dir: b.dir}, regAt(5), func(r *leadRegistry) {
		job, err := r.get("job-1")
		if err != nil {
			t.Fatalf("get job-1 after reload: %v", err)
		}
		if job.State != JobRunning || job.TaskID != "task-1" || job.Branch != "branch-1" {
			t.Fatalf("reloaded job fields mismatch: %+v", job)
		}
		if len(job.History) != 5 {
			t.Fatalf("reloaded history length = %d, want 5: %+v", len(job.History), job.History)
		}
		for i, want := range append([]JobTransition{
			{State: JobQueued, At: regAt(0)},
			{State: JobAssigned, At: regAt(1)},
		}, transitions...) {
			if job.History[i].State != want.State || !job.History[i].At.Equal(want.At.Time) {
				t.Fatalf("history[%d] = %+v, want %+v", i, job.History[i], want)
			}
		}

		// Transition to terminal state (done)
		_, errDone := r.report(JobReport{
			ID:         "job-1",
			Node:       "node-a",
			Assignment: assignment,
			State:      JobDone,
			TaskID:     "task-1",
			Branch:     "branch-1",
			Result:     "success",
			At:         regAt(6),
		}, regAt(6))
		if errDone != nil {
			t.Fatalf("report done: %v", errDone)
		}
	})

	// Reopen once more to verify terminal outcome survived
	storeRun(t, &Bus{dir: b.dir}, regAt(7), func(r *leadRegistry) {
		job, err := r.get("job-1")
		if err != nil {
			t.Fatalf("get terminal job: %v", err)
		}
		if job.State != JobDone || job.Result != "success" || len(job.History) != 6 {
			t.Fatalf("terminal job mismatch after reload: %+v", job)
		}
	})
}
