package main

// A running agent, a configured peer or a worker of a request is known to the bus outside teams, but that never lets a
// direct message past the agent's open teams: the message is bound to one of them or refused.

import (
	"os"
	"strings"
	"testing"
)

// runningIn registers a live presence of the agent (this test's own process), as its run in team does.
func runningIn(t *testing.T, b *Bus, agent, team string) {
	t.Helper()
	chk(t).ok(toolResult(server(b, agent, inTeam(team)).read(0)))
	mustWith(t, b, func(st *State) {
		if p := st.Agents[agent]; p == nil || p.PID != os.Getpid() {
			t.Fatalf("no live presence of %s: %+v", agent, p)
		}
	})
}

func TestAnOutsideMessageToARunningAgentInTwoTeamsIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	runningIn(t, b, "codex-1", "A")
	woken := captureWakes(t)

	err := failure(t, SendMessageAmbiguousAgent)(server(b, "inv-1").sendMessage("codex-1", "reset your branch", true))

	if !strings.Contains(err.Message, "task-codex-1-a") || !strings.Contains(err.Message, "task-codex-1-b") {
		t.Fatalf("the refusal must name both tasks, got: %s", err.Message)
	}
	if len(*woken) != 0 || strings.Contains(allMessages(t, b), "reset your branch") {
		t.Fatalf("a refused message woke %v or was stored", *woken)
	}
}

func TestAMessageToARunningOrConfiguredMemberOfAnotherTeamIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	runningIn(t, b, "lead-b", "B")
	woken := captureWakes(t)
	lead := server(b, "lead-a", inTeam("A"), func(s *Server) { s.peers = []string{"lead-a", "lead-b", "codex-1"} })

	err := failure(t, SendMessageNotInYourTeam)(lead.sendMessage("lead-b", "take item 1.1", true))

	if !strings.Contains(err.Message, "codex-1") || !strings.Contains(err.Message, "lead-a") {
		t.Fatalf("the refusal must list the members of the team, got: %s", err.Message)
	}
	if len(*woken) != 0 || strings.Contains(allMessages(t, b), "take item 1.1") {
		t.Fatalf("a refused message woke %v or was stored", *woken)
	}
}

func TestAHelperMessageToARunningRequesterIsBoundToTheTaskOfItsOpenRequest(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	runningIn(t, b, "codex-1", "A")
	mustWith(t, b, func(st *State) {
		st.Helpers["1"] = &Helper{ID: 1, From: "codex-1", FromTask: "task-codex-1-b", Repo: "repo",
			Branch: "agents/codex-1", Worktree: "/tmp/wt-codex-1", Request: "list the files", Status: HelperRunning, Worker: "worker-1"}
		st.Helpers["2"] = &Helper{ID: 2, From: "codex-1", FromTask: "task-codex-1-a", Repo: "repo",
			Branch: "agents/codex-1", Worktree: "/tmp/wt-codex-1", Request: "an old request", Status: HelperDone, Worker: "worker-1"}
	})
	woken := captureWakes(t)

	chk(t).ok(toolResult(server(b, "worker-1").send("codex-1", "the files are a.go and b.go")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-b" {
		t.Fatalf("woken %v, want only the task of the open request", got)
	}
	if out := readOutput(t, b, "codex-1", "A"); strings.Contains(out, "the files are a.go") {
		t.Fatalf("the requester's other team read the helper's message:\n%s", out)
	}
	if out := readOutput(t, b, "codex-1", "B"); !strings.Contains(out, "the files are a.go") {
		t.Fatalf("the task of the request did not read the helper's message:\n%s", out)
	}
}

func TestASessionWhoseTeamIsNotOnTheBusWritesAsFromOutsideTeams(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)
	woken := captureWakes(t)
	solo := server(b, "claude-1", inTeam("team-solo"))

	failsWith(t, SendMessageAmbiguousAgent)(solo.sendMessage("codex-1", "which team?", true))
	chk(t).ok(toolResult(solo.sendMessage("human", "a question for you", true)))

	if len(*woken) != 0 {
		t.Fatalf("woken %v", *woken)
	}
}
