package main

// Tests of progress notes and members out (written first): a note stays with an item that goes back to the board,
// with the member that held it; only the claimant writes notes; member_out is a notice of the agent script.

import (
	"strings"
	"testing"
)

func TestBoardProgressSurvivesARelease(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}, "w": {"worker", "c", "active"}})
	lead, m, w := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T")), server(b, "w", inTeam("T"), asWorker)
	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.boardAdd("parser", "write the parser", "easy")))
	chk(t).ok(toolResult(w.boardClaim("B1")))
	failsWith(t, BoardProgressNotYourItem)(m.boardProgress("B1", "not mine"))
	failsWith(t, BoardProgressEmptyNote)(w.boardProgress("B1", "  "))
	chk(t).ok(toolResult(w.boardProgress("B1", "lexer done at abc123; next: the parser")))
	mustWith(t, b, func(st *State) {
		if got := st.Teams["T"].releaseItemsOf("w"); len(got) != 1 {
			t.Fatalf("released %v, want B1", got)
		}
	})
	out := chk(t).ok(toolResult(m.boardList()))
	for _, want := range []string{"lexer done at abc123", "previously: w"} {
		if !strings.Contains(out, want) {
			t.Errorf("board_list lacks %q:\n%s", want, out)
		}
	}
}

func TestMemberOutIsANotice(t *testing.T) {
	if !ScriptNotice("member_out").Valid() {
		t.Error("member_out is not a valid script notice")
	}
}
