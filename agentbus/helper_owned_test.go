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

func TestOwnedRequestRetryFencesTheEarlierExecution(t *testing.T) {
	b, cmd := ownedFixture(t)
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("start = %s, %v", got, err)
	}
	cmd.Action = "retry"
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("retry = %s, %v", got, err)
	}
	mustWith(t, b, func(st *State) { st.Helpers["1"].Status, st.Helpers["1"].Worker = HelperStarting, "other" })
	late := cmd
	cmd.Action, cmd.Worker, cmd.Run = "start", "other", "run-2"
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("new start = %s, %v", got, err)
	}
	late.Action, late.Status, late.Notice = "settle", HelperFailed, "late failure"
	if got, err := b.helperOwned(late); err != nil || got != ownedStale {
		t.Fatalf("late = %s, %v", got, err)
	}
	cmd.Action, cmd.Status, cmd.Notice = "settle", HelperFailed, "other returned step 1.2"
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("final = %s, %v", got, err)
	}
}

func TestOwnedCommandErrorsKeepTheirCauses(t *testing.T) {
	for _, tc := range []struct {
		name   string
		change func(*ownedCommand)
		cause  CmdHelperOwnedCause
	}{
		{"action", func(c *ownedCommand) { c.Action = "unknown" }, CmdHelperOwnedInvalidArgument},
		{"run", func(c *ownedCommand) { c.Run = "" }, CmdHelperOwnedInvalidArgument},
		{"worker", func(c *ownedCommand) { c.Worker = "" }, CmdHelperOwnedInvalidArgument},
		{"task", func(c *ownedCommand) { c.Task = "" }, CmdHelperOwnedInvalidArgument},
		{"status", func(c *ownedCommand) { c.Action, c.Status, c.Notice = "settle", HelperQueued, "notice" }, CmdHelperOwnedInvalidArgument},
		{"notice", func(c *ownedCommand) { c.Action, c.Status = "settle", HelperDone }, CmdHelperOwnedInvalidArgument},
		{"missing", func(c *ownedCommand) { c.ID = 42 }, CmdHelperOwnedNoSuchRequest},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b, cmd := ownedFixture(t)
			tc.change(&cmd)
			_, err := b.helperOwned(cmd)
			failsWith(t, tc.cause)("helper-owned", err)
		})
	}
}

func TestOwnedReceiptRepairsAnInterruptedStateSave(t *testing.T) {
	b, cmd := ownedFixture(t)
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("start = %s, %v", got, err)
	}
	before, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	cmd.Action, cmd.Status, cmd.Summary, cmd.Notice = "settle", HelperDone, "kept", "worker served the step"
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("settle = %s, %v", got, err)
	}
	// Simulate a crash after the notice append but before state.json is replaced.
	if serr := b.save(before); serr != nil {
		t.Fatal(serr)
	}
	restarted := &Bus{dir: b.dir}
	loaded, serr := restarted.load()
	if serr != nil {
		t.Fatal(serr)
	}
	if h := loaded.Helpers["1"]; h.Status != HelperDone || h.Summary != "kept" {
		t.Fatalf("reloaded receipt = %+v", h)
	}
	if got, err := restarted.helperOwned(cmd); err != nil || got != ownedReplay {
		t.Fatalf("replay = %s, %v", got, err)
	}
	messages, serr := restarted.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(messages) != 1 || loaded.NextID <= messages[0].ID {
		t.Fatalf("repaired messages/cursor = %+v / %d", messages, loaded.NextID)
	}
}

func TestOwnedCommandReportsStorageFailure(t *testing.T) {
	b, cmd := ownedFixture(t)
	// A child of state.json cannot be a directory, so opening the bus lock fails.
	broken := &Bus{dir: b.path("state.json/child")}
	_, err := broken.helperOwned(cmd)
	failsWith(t, CmdHelperOwnedStorage)("helper-owned", err)
}

func TestOwnedOperationLeavesLegacyOpenRequestsUnfenced(t *testing.T) {
	b := installState(t, helperBeforeWorkerResult)
	st, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	if h := st.Helpers["1"]; h == nil || h.Status != HelperQueued || h.Step != "1.1" {
		t.Fatalf("old request = %+v", h)
	}
}
