package main

import "testing"

func TestOwnedQueuedExhaustionSettlesOnceAndRejectsChangedOwner(t *testing.T) {
	b, cmd := ownedFixture(t)
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("start = %s, %v", got, err)
	}
	cmd.Action = ownedRetry
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("retry = %s, %v", got, err)
	}
	cmd.Action, cmd.Status, cmd.Notice = ownedSettle, HelperFailed, "worker returned exhausted request"
	late := cmd
	late.Run = "old-run"
	if got, err := b.helperOwned(late); err != nil || got != ownedStale {
		t.Fatalf("old owner = %s, %v", got, err)
	}
	late = cmd
	late.Status = HelperDone
	if got, err := b.helperOwned(late); err != nil || got != ownedStale {
		t.Fatalf("late success = %s, %v", got, err)
	}
	for _, want := range []ownedOutcome{ownedApplied, ownedReplay} {
		if got, err := b.helperOwned(cmd); err != nil || got != want {
			t.Fatalf("queued settlement = %s, %v; want %s", got, err, want)
		}
	}
	messages, err := b.messages()
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || messages[0].Text != cmd.Notice {
		t.Fatalf("final notices = %+v", messages)
	}
}
