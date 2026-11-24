package main

import "testing"

func TestPoolQueueReservesOnlyRequestsWithoutLegacyWorkers(t *testing.T) {
	for _, task := range []string{"task-m", ""} {
		t.Run("legacy task="+task, func(t *testing.T) {
			st := &State{
				Teams: map[string]*Team{"T": teamOf(map[string][2]string{
					"m": {"normal", "active"}, "w-old": {"worker", "sleeping"},
				})},
				Helpers: map[string]*Helper{"1": {ID: 1, From: "m", FromTask: task, Status: HelperQueued}},
			}
			if got := freePoolWorkersState(st, []string{"w-old", "w-free"}); got != 1 {
				t.Fatalf("legacy-owned queue consumed pool capacity: got %d, want 1", got)
			}
			st.Helpers["2"] = &Helper{ID: 2, From: "m", FromTask: "task-m-other", Status: HelperQueued}
			if got := freePoolWorkersState(st, []string{"w-old", "w-free"}); got != 0 {
				t.Fatalf("request from another task must reserve pool capacity: got %d, want 0", got)
			}
		})
	}
}
