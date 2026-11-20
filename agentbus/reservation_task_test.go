package main

import (
	"strings"
	"testing"
)

// TestAReservationBelongsToTheTaskOfTheAgentThatTookIt: an agent in two open teams has a run in each; the run of one
// task neither finds the other's reservation its own, nor releases it, nor frees it when it ends.
func TestAReservationBelongsToTheTaskOfTheAgentThatTookIt(t *testing.T) {
	b := newBus(t)
	first := server(b, "codex-1")
	second := server(b, "codex-1", func(s *Server) { s.info.Task = "task-codex-1-b" })
	if out := chk(t).ok(toolResult(first.reserve("probe-1", "flash", 0))); !strings.HasPrefix(out, "OK: reserved") {
		t.Fatalf("reserve: %s", out)
	}

	if out := chk(t).ok(toolResult(second.reserve("probe-1", "", 0))); !strings.HasPrefix(out, "TAKEN") {
		t.Fatalf("the other task's reserve should report TAKEN, got: %s", out)
	}
	failsWith(t, ReleaseNotHolder)(second.release("probe-1"))
	second.unregister()

	mustWith(t, b, func(st *State) {
		if r := st.Reservations["probe-1"]; r == nil || r.Task != "task-codex-1" {
			t.Fatalf("the reservation of task-codex-1 is gone or moved: %+v", r)
		}
	})
}
