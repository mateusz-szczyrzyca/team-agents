package main

// `agentbus send --for-task T` writes to the session of the task T alone: the daemon names the task of `agent say`, a
// solo task among them, which no team of the bus holds.

import (
	"strings"
	"testing"
)

// soloTask is a task of codex-1 that is in no team of the bus, as a solo task is.
const soloTask = "task-codex-1-solo"

// sendFor posts a chat as `agentbus send` does for req and fails the test on a storage error.
func sendFor(t *testing.T, b *Bus, req sendRequest, text string) *CmdSendError {
	t.Helper()
	var refusal *CmdSendError
	mustWith(t, b, func(st *State) {
		_, refusal = postSend(b, st, req, EventChat{Body: text})
	})
	return refusal
}

// readIn is what the session of codex-1 with task (in team, "" for none) reads now.
func readIn(t *testing.T, b *Bus, team, task string) string {
	t.Helper()
	return chk(t).ok(toolResult(server(b, "codex-1", inTeam(team), func(s *Server) { s.info.Task = task }).read(0)))
}

func TestASendForATaskOfNoTeamIsReadOnlyByThatTasksSession(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	woken := captureWakes(t)

	if refusal := sendFor(t, b, sendRequest{from: "human", to: "codex-1", forTask: soloTask}, "only for the solo task"); refusal != nil {
		t.Fatal(refusal)
	}

	if out := readIn(t, b, "A", "task-codex-1-a"); strings.Contains(out, "only for the solo task") {
		t.Fatalf("team A read the message of the solo task:\n%s", out)
	}
	if out := readIn(t, b, "B", "task-codex-1-b"); strings.Contains(out, "only for the solo task") {
		t.Fatalf("team B read the message of the solo task:\n%s", out)
	}
	if out := readIn(t, b, "team-solo", soloTask); !strings.Contains(out, "only for the solo task") {
		t.Fatalf("the solo task did not read its message:\n%s", out)
	}
	if len(*woken) != 0 {
		t.Fatalf("woken %v", *woken)
	}
}

func TestASendForATaskOfATeamWakesAndIsReadOnlyByThatTask(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	woken := captureWakes(t)

	if refusal := sendFor(t, b, sendRequest{from: "human", to: "codex-1", forTask: "task-codex-1-b"}, "for team B"); refusal != nil {
		t.Fatal(refusal)
	}

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-b" {
		t.Fatalf("woken %v, want only task-codex-1-b", got)
	}
	if out := readIn(t, b, "A", "task-codex-1-a"); strings.Contains(out, "for team B") {
		t.Fatalf("team A read the message of team B:\n%s", out)
	}
	if out := readIn(t, b, "B", "task-codex-1-b"); !strings.Contains(out, "for team B") {
		t.Fatalf("team B did not read its message:\n%s", out)
	}
}

func TestASendForTheTaskOfAnotherMemberIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	captureWakes(t)

	refusal := sendFor(t, b, sendRequest{from: "human", to: "codex-1", forTask: "task-lead-b"}, "not yours")

	if refusal == nil || refusal.Cause != CmdSendNoSuchMember {
		t.Fatalf("got %v, want NoSuchMember", refusal)
	}
}

func TestASendForATaskWithATaskOrTeamIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	captureWakes(t)

	for _, req := range []sendRequest{
		{from: "human", to: "codex-1", forTask: soloTask, task: "task-codex-1-a"},
		{from: "human", to: "codex-1", forTask: soloTask, team: "A"},
		{from: "human", to: "all", forTask: soloTask},
	} {
		if refusal := sendFor(t, b, req, "two bindings"); refusal == nil || refusal.Cause != CmdSendTwoBindings {
			t.Fatalf("%+v: got %v, want TwoBindings", req, refusal)
		}
	}
}

func TestTheHistoryOfATeamShowsNothingForTheAgentsSoloTask(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	captureWakes(t)
	if refusal := sendFor(t, b, sendRequest{from: "human", to: "codex-1", forTask: soloTask}, "only for the solo task"); refusal != nil {
		t.Fatal(refusal)
	}

	inA := chk(t).ok(toolResult(server(b, "codex-1", inTeam("A"), func(s *Server) { s.info.Task = "task-codex-1-a" }).read(50)))

	if strings.Contains(inA, "only for the solo task") {
		t.Fatalf("the history of team A shows the solo task's message:\n%s", inA)
	}
}
