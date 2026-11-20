package main

// The CLI subcommand `agentbus send` for the daemon: --task and --team name the open team an agent is written to in,
// since the daemon's own session is in no team.

import (
	"strings"
	"testing"
)

func TestSendWithATaskBindsTheMessageToTheTeamOfThatTask(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	recipient, team, err := bindingIn(b, "codex-1", sendWhere{task: "task-codex-1-b"})

	if err != nil {
		t.Fatalf("binding: %v", err)
	}
	if recipient != "codex-1" || team != "B" {
		t.Fatalf("bound to %q in %q, want codex-1 in team B", recipient, team)
	}
	if len(*woken) != 0 {
		t.Fatalf("binding woke %v", *woken)
	}
}

func TestSendWithATeamBindsTheMessageToIt(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)

	recipient, team, err := bindingIn(b, "codex-1", sendWhere{team: "A"})

	if err != nil {
		t.Fatalf("binding: %v", err)
	}
	if recipient != "codex-1" || team != "A" {
		t.Fatalf("bound to %q in %q, want codex-1 in team A", recipient, team)
	}
}

func TestSendToANameInTwoTeamsWithoutATaskOrTeamIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)

	_, _, err := bindingIn(b, "codex-1", sendWhere{})

	if err == nil || err.Cause != CmdSendAmbiguousAgent {
		t.Fatalf("got %v, want AmbiguousAgent", err)
	}
	if !strings.Contains(err.Message, "task-codex-1-a") || !strings.Contains(err.Message, "task-codex-1-b") {
		t.Fatalf("the refusal must name every task, got: %s", err.Message)
	}
}

func TestSendWithATaskNoMemberOfTheTeamHasIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)

	_, _, err := bindingIn(b, "codex-1", sendWhere{task: "task-nobody"})
	if err == nil || err.Cause != CmdSendNoSuchMember {
		t.Fatalf("an unknown task: got %v, want NoSuchMember", err)
	}
	// a task of the agent in the other open team: --team names the team it is not in
	_, _, err = bindingIn(b, "codex-1", sendWhere{task: "task-codex-1-a", team: "B"})
	if err == nil || err.Cause != CmdSendNoSuchMember {
		t.Fatalf("a task of another team: got %v, want NoSuchMember", err)
	}
}

func TestSendWithATaskOrTeamToEveryoneIsRefused(t *testing.T) {
	b := newBus(t)
	twoTeams(t, b)

	for _, to := range []string{"all", "team:A"} {
		_, _, err := bindingIn(b, to, sendWhere{task: "task-codex-1-a"})
		if err == nil || err.Cause != CmdSendBindingForMany {
			t.Fatalf("%s: got %v, want an invalid argument", to, err)
		}
	}
}

func TestSendWithNeitherATaskNorATeamFollowsTheAgentsOneOpenTeam(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "A", map[string][3]string{"lead-a": {"normal", "", "active"}, "codex-1": {"normal", "", "sleeping"}})

	recipient, team, err := bindingIn(b, "codex-1", sendWhere{})

	if err != nil || recipient != "codex-1" || team != "A" {
		t.Fatalf("got %q in %q, %v — want the agent's one open team", recipient, team, err)
	}
}

// sendWhere is what `agentbus send` says about the team a message goes to: a task of a member, a team, or neither.
type sendWhere struct {
	task string // --task
	team string // --team
}

// bindingIn runs the binding of send against the bus, as the command does, and closes the bus afterwards.
func bindingIn(b *Bus, to string, where sendWhere) (string, string, *CmdSendError) {
	var recipient, team string
	var failure *CmdSendError
	serr := b.locked(func(st *State) {
		recipient, team, failure = bindSend(st, "human", to, where.task, where.team)
	})
	if serr != nil {
		panic(serr)
	}
	return recipient, team, failure
}
