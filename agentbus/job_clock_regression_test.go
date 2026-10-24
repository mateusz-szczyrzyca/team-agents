package main

import "testing"

// TestLeadOwnedTransitionsSurviveClockRegression keeps successful writes readable when a lead clock moves back.
func TestLeadOwnedTransitionsSurviveClockRegression(t *testing.T) {
	for _, operation := range []string{"cancel-queued", "cancel-assigned", "assign", "requeue"} {
		t.Run(operation, func(t *testing.T) {
			b := newBus(t)
			storeRun(t, b, regAt(10), func(r *leadRegistry) {
				regSubmit(t, r, regRequest("clock"), regAt(10))
				if operation == "cancel-assigned" || operation == "requeue" {
					if regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(11)) == nil {
						t.Fatal("fixture not assigned")
					}
				}
			})
			storeRun(t, b, regAt(9), func(r *leadRegistry) {
				switch operation {
				case "cancel-queued", "cancel-assigned":
					if _, err := r.cancel("clock", regAt(9)); err != nil {
						t.Fatal(err)
					}
				case "assign":
					if regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(9)) == nil {
						t.Fatal("job not assigned")
					}
				case "requeue":
					// A missing node must not strand its unstarted job even when receipt time is earlier.
					delete(r.nodes, "node-a")
					if len(r.requeueOffline(regAt(9), 2*regTTL)) != 1 {
						t.Fatal("job not requeued")
					}
				}
			})
			storeRun(t, &Bus{dir: b.dir}, regAt(12), func(r *leadRegistry) {
				job := r.jobs["clock"]
				last := job.History[len(job.History)-1]
				previous := job.History[len(job.History)-2]
				if !last.At.Equal(previous.At.Time) {
					t.Fatalf("backward lead time must clamp to previous transition: %+v", job.History)
				}
			})
		})
	}
}
