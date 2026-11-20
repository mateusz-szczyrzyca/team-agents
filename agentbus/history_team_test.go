package main

// The history of read_messages and read_message by number show a session nothing of the agent's other team: they are
// reads like read_messages itself.

import (
	"strings"
	"testing"
)

// lastID is the number of the last message on the bus.
func lastID(t *testing.T, b *Bus) int {
	t.Helper()
	msgs, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(msgs) == 0 {
		t.Fatal("no message on the bus")
	}
	return int(msgs[len(msgs)-1].ID)
}

func TestTheHistoryOfOneTeamShowsNothingOfTheAgentsOtherTeam(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	twoTeams(t, b)
	leadB := server(b, "lead-b", inTeam("B"))
	chk(t).ok(toolResult(leadB.send("codex-1", "team B only")))
	chk(t).ok(toolResult(leadB.send("team", "a notice of team B")))

	inA := chk(t).ok(toolResult(server(b, "codex-1", inTeam("A")).read(50)))
	inB := chk(t).ok(toolResult(server(b, "codex-1", inTeam("B")).read(50)))

	if strings.Contains(inA, "team B only") || strings.Contains(inA, "a notice of team B") {
		t.Fatalf("the history of team A shows team B's messages:\n%s", inA)
	}
	if !strings.Contains(inB, "team B only") || !strings.Contains(inB, "a notice of team B") {
		t.Fatalf("the history of team B lacks its messages:\n%s", inB)
	}
}

func TestAMessageOfTheAgentsOtherTeamIsNotReadByItsNumber(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	twoTeams(t, b)
	leadB := server(b, "lead-b", inTeam("B"))
	chk(t).ok(toolResult(leadB.send("codex-1", "team B only")))
	short := lastID(t, b)
	chk(t).ok(toolResult(leadB.send("codex-1", strings.Repeat("ł", messageTextMax+10))))
	long := lastID(t, b)

	for _, id := range []int{short, long} {
		failsWith(t, ReadMessageNoSuchMessage)(server(b, "codex-1", inTeam("A")).readMessage(id))
		chk(t).ok(toolResult(server(b, "codex-1", inTeam("B")).readMessage(id)))
	}
}
