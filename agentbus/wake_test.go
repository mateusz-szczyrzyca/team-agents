package main

import "testing"

// teamSet runs the team-set subcommand for one member.
func teamSet(t *testing.T, b *Bus, team, agent string, state MemberState) {
	t.Helper()
	cmdTeamSet([]string{"--dir", b.dir, "--team", team, "--agent", agent, "--state", string(state)})
}

// memberOf returns a copy of a member's current entry.
func memberOf(t *testing.T, b *Bus, team, agent string) TeamMember {
	t.Helper()
	var m TeamMember
	mustWith(t, b, func(st *State) { m = *st.Teams[team].Members[agent] })
	return m
}

// readAll stands for the member's run reading its messages (so that only the board can wake it).
func readAll(t *testing.T, s *Server) {
	t.Helper()
	chk(t).ok(toolResult(s.read(0)))
}

// wakeCount counts the wake-ups of a task.
func wakeCount(woken *[]string, task string) int {
	n := 0
	for _, w := range *woken {
		if w == task {
			n++
		}
	}
	return n
}

// TestBoardDoesNotWakeForItemsAlreadySeen reproduces the wake loop: a member that leaves an open item alone
// (here one the coordinator keeps for later) must not be woken again every time it goes to sleep.
func TestBoardDoesNotWakeForItemsAlreadySeen(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead, m := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.boardAdd("later", "the coordinator assigns this one later", "normal")))
	readAll(t, m)

	// m has not looked at the board yet: going to sleep wakes it once
	teamSet(t, b, "T", "m", MemberSleeping)
	if n := wakeCount(woken, "task-m"); n != 1 {
		t.Fatalf("woken %d times, want 1", n)
	}
	// the same board does not wake it again, however many times it goes to sleep
	for i := 0; i < 5; i++ {
		readAll(t, m)
		teamSet(t, b, "T", "m", MemberSleeping)
	}
	if n := wakeCount(woken, "task-m"); n != 1 {
		t.Fatalf("woken %d times by the same board, want 1", n)
	}
	if st := memberOf(t, b, "T", "m").State; st != MemberSleeping {
		t.Fatalf("state %q, want sleeping", st)
	}
	// a new item wakes it again — once
	chk(t).ok(toolResult(lead.boardAdd("now", "do it", "normal")))
	readAll(t, m)
	teamSet(t, b, "T", "m", MemberSleeping)
	if n := wakeCount(woken, "task-m"); n != 2 {
		t.Fatalf("woken %d times after a new item, want 2", n)
	}
}

func TestBoardListMarksTheBoardSeen(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead, m := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T"))
	chk(t).ok(toolResult(lead.boardAdd("open", "anyone", "normal")))
	chk(t).ok(toolResult(m.boardList()))
	teamSet(t, b, "T", "m", MemberSleeping)
	if n := wakeCount(woken, "task-m"); n != 0 {
		t.Fatalf("a member that listed the board was woken %d times for it", n)
	}
}

func TestReturnedItemsCountAsNewWork(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead, m := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T"))
	chk(t).ok(toolResult(lead.boardAdd("one", "d", "normal")))
	chk(t).ok(toolResult(m.boardClaim("B1")))
	rev := func() int {
		var r int
		mustWith(t, b, func(st *State) { r = st.Teams["T"].BoardRev })
		return r
	}
	before := rev()
	chk(t).ok(toolResult(m.boardRelease("B1", "cannot do it")))
	if rev() != before+1 {
		t.Fatalf("board_release must count as new work: rev %d → %d", before, rev())
	}
	chk(t).ok(toolResult(m.boardClaim("B1")))
	before = rev()
	mustWith(t, b, func(st *State) { st.Teams["T"].releaseItemsOf("m") })
	if rev() != before+1 {
		t.Fatalf("items released from a hung member must count as new work: rev %d → %d", before, rev())
	}
}

func TestDesignatedBoardItems(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{
		"lead": {"normal", "a", "active"}, "m1": {"normal", "b", "sleeping"},
		"m2": {"normal", "c", "sleeping"}, "w": {"worker", "d", "sleeping"},
	})
	lead, m1, m2 := server(b, "lead", inTeam("T")), server(b, "m1", inTeam("T")), server(b, "m2", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	// the "coordinator claimed" team message woke the sleepers; they go back to sleep after reading it
	for _, s := range []*Server{m1, m2, server(b, "w", inTeam("T"))} {
		readAll(t, s)
		teamSet(t, b, "T", s.me, MemberSleeping)
	}
	*woken = (*woken)[:0]

	chk(t).ok(toolResult(lead.boardAddFor("for m2", "only m2", "normal", "m2")))
	if wakeCount(woken, "task-m2") != 1 || wakeCount(woken, "task-m1") != 0 {
		t.Fatalf("a designated item must wake only its member: %v", *woken)
	}
	failsWith(t, BoardClaimReservedForOther)(m1.boardClaim("B1"))
	chk(t).ok(toolResult(m2.boardClaim("B1")))

	failsWith(t, BoardAddUnknownMember)(lead.boardAddFor("x", "d", "normal", "ghost"))
	failsWith(t, BoardAddForWorkerNeedsEasy)(lead.boardAddFor("x", "d", "normal", "w"))

	// an item reserved for someone else is no work for m1: going to sleep does not wake it
	chk(t).ok(toolResult(lead.boardAddFor("also for m2", "only m2", "normal", "m2")))
	readAll(t, m1)
	teamSet(t, b, "T", "m1", MemberSleeping)
	if n := wakeCount(woken, "task-m1"); n != 0 {
		t.Fatalf("m1 was woken %d times for items reserved for m2", n)
	}
}

func TestPausedMemberIsNotWoken(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	teamSet(t, b, "T", "m", MemberPaused)
	chk(t).ok(toolResult(lead.send("m", "are you there?")))
	chk(t).ok(toolResult(lead.boardAdd("open", "anyone", "normal")))
	teamSet(t, b, "T", "m", MemberPaused)
	if n := wakeCount(woken, "task-m"); n != 0 {
		t.Fatalf("a paused member was woken %d times", n)
	}
	if st := memberOf(t, b, "T", "m").State; st != MemberPaused {
		t.Fatalf("state %q, want paused", st)
	}
}
