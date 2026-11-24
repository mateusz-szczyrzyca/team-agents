package main

import "testing"

// ownedFixture creates a claimed request without changing any existing helper fixtures.
func ownedFixture(t *testing.T) (*Bus, ownedCommand) {
	t.Helper()
	b := newBus(t)
	mustWith(t, b, func(st *State) {
		st.Helpers["1"] = &Helper{ID: 1, From: "lead", FromTask: "requester-task", Worker: "worker", Status: HelperStarting}
	})
	return b, ownedCommand{ID: 1, Action: "start", Worker: "worker", Task: "worker-task", Run: "run-1"}
}

func TestOwnedSettlementPostsOneNoticeAndRejectsLateResults(t *testing.T) {
	b, cmd := ownedFixture(t)
	got, err := b.helperOwned(cmd)
	if err != nil || got != ownedApplied {
		t.Fatalf("start = %s, %v", got, err)
	}
	cmd.Action, cmd.Status, cmd.Summary, cmd.Notice = "settle", HelperDone, "accepted", "worker completed step 1.2"
	for _, want := range []ownedOutcome{ownedApplied, ownedReplay} {
		got, err = b.helperOwned(cmd)
		if err != nil || got != want {
			t.Fatalf("settle = %s, %v; want %s", got, err, want)
		}
	}
	cmd.Run, cmd.Summary = "late-run", "must not replace"
	got, err = b.helperOwned(cmd)
	if err != nil || got != ownedStale {
		t.Fatalf("late = %s, %v", got, err)
	}
	mustWith(t, b, func(st *State) {
		if h := st.Helpers["1"]; h.Status != HelperDone || h.Summary != "accepted" {
			t.Fatalf("settled request = %+v", h)
		}
	})
	messages, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(messages) != 1 || messages[0].Task != "requester-task" || messages[0].Text != "worker completed step 1.2" {
		t.Fatalf("final notices = %+v", messages)
	}
}
