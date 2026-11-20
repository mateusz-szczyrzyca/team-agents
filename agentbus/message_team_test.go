package main

// A direct message is for the named agent in the team of its sender: it wakes only that team's task and is read only by
// that team's session. An agent may be in two open teams at once, with a task and a worktree in each; nothing sent in one
// of them may reach the other (the incident of 19.11.2026: the lead of a second team wrote to an agent, the session of the
// first team woke up and hard-reset the branch of the second team's task).

import (
	_ "embed"
	"os"
	"strings"
	"testing"
)

// legacyMessageTeamState is a bus state written before messages named their team, as 0.28.0 writes one: one team "T"
// with its lead and a sleeping member, the lead's read cursor past the team's creation and the coordinator notice, and
// the member's direct message of the lead unread.
//
//go:embed testdata/state-before-message-team.json
var legacyMessageTeamState []byte

// legacyMessageTeamLog is the message log of that state: the team's creation and coordinator notice addressed to the
// team, the lead's direct message to the member, and the member's broadcast.
//
//go:embed testdata/messages-before-message-team.jsonl
var legacyMessageTeamLog []byte

// installMessages writes a message log into a bus directory (its state being installed already).
func installMessages(t *testing.T, b *Bus, log []byte) {
	t.Helper()
	if err := os.WriteFile(b.path("messages.jsonl"), log, 0o600); err != nil {
		t.Fatal(err)
	}
}

// legacyBus installs the state and the message log of a bus written before messages named their team.
func legacyBus(t *testing.T) *Bus {
	t.Helper()
	b := installState(t, legacyMessageTeamState)
	installMessages(t, b, legacyMessageTeamLog)
	return b
}

// TestMessagesBeforeTheTeamOfASenderLoad: a state and a log written before messages named their team load as they are:
// the team, its members and their tasks, and the cursors of the agents.
func TestMessagesBeforeTheTeamOfASenderLoad(t *testing.T) {
	b := legacyBus(t)

	mustWith(t, b, func(st *State) {
		if len(st.Teams) != 1 || st.Teams["T"] == nil {
			t.Fatalf("the team did not load: %v", st.Teams)
		}
		team := st.Teams["T"]
		if team.Members["member"] == nil || team.Members["member"].Task != "task-member" || team.Members["member"].State != MemberSleeping {
			t.Fatalf("the member did not load as written: %+v", team.Members["member"])
		}
		if st.Cursors["lead"] != 2 {
			t.Fatalf("the cursor of lead did not load: %v", st.Cursors)
		}
	})
	msgs, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(msgs) != 3 || msgs[2].To != "member" || msgs[2].Text != "please take B1" {
		t.Fatalf("the old messages did not load as written: %v", msgs)
	}
}

// twoTeams sets up the agent "codex-1" in two open teams, each with its own lead and task: "A" (lead-a) and "B"
// (lead-b), made in that order, with the tasks an installation gives two teams of one agent.
func twoTeams(t *testing.T, b *Bus) {
	t.Helper()
	for _, team := range []string{"A", "B"} {
		lead := "lead-" + strings.ToLower(team)
		members := map[string]*TeamMember{}
		for _, m := range []*TeamMember{
			{Agent: lead, Task: "task-" + lead, Tier: TierNormal, State: MemberActive},
			{Agent: "codex-1", Task: "task-codex-1-" + strings.ToLower(team), Tier: TierNormal, State: MemberSleeping},
		} {
			members[m.Agent] = m
		}
		mustWith(t, b, func(st *State) { st.Teams[team] = &Team{ID: team, Repo: "repo", Members: members} })
	}
}

func TestADirectMessageWakesOnlyTheTaskOfTheSendersTeam(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	chk(t).ok(toolResult(server(b, "lead-b", inTeam("B")).send("codex-1", "take item 1.2 of team B")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-b" {
		t.Fatalf("woken %v, want only the task of team B", got)
	}
}

func TestADirectMessageIsReadOnlyInTheSendersTeam(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	twoTeams(t, b)
	chk(t).ok(toolResult(server(b, "lead-b", inTeam("B")).send("codex-1", "take item 1.2 of team B")))

	// what team A's session would read, without moving its cursor: the message of team B is not for it
	mustWith(t, b, func(st *State) {
		msgs, serr := b.messages()
		if serr != nil {
			t.Fatal(serr)
		}
		for _, m := range msgs {
			if isForAgent(st, m, "codex-1", "A", true) {
				t.Fatalf("team A has the message meant for team B: %s", fmtMsg(m))
			}
			if !isForAgent(st, m, "codex-1", "B", true) {
				t.Fatalf("team B does not have its message: %s", fmtMsg(m))
			}
		}
	})
}

func TestAMessageToTheTeamIsReadAndWokenAsBefore(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	chk(t).ok(toolResult(server(b, "lead-b", inTeam("B")).send("team", "status meeting of team B")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-b" {
		t.Fatalf("woken %v, want only the task of team B", got)
	}
	mustWith(t, b, func(st *State) {
		msgs, serr := b.messages()
		if serr != nil {
			t.Fatal(serr)
		}
		for _, m := range msgs {
			if !isForAgent(st, m, "codex-1", "B", true) || isForAgent(st, m, "codex-1", "A", true) {
				t.Fatalf("%s: for A, want for B only", fmtMsg(m))
			}
		}
	})
}

func TestAMessageWithoutATeamIsForTheNamedAgentInTheTeamItIsReadIn(t *testing.T) {
	b := legacyBus(t)

	// a direct message without a team (an older one) and the team's notices are for the member of the team
	mustWith(t, b, func(st *State) {
		msgs, serr := b.messages()
		if serr != nil {
			t.Fatal(serr)
		}
		if got := len(msgs); !isForAgent(st, msgs[got-1], "member", "T", true) {
			t.Fatalf("the old direct message is not for the member of its team: %s", fmtMsg(msgs[got-1]))
		}
	})
	if out := readOutput(t, b, "member", "T"); !strings.Contains(out, "please take B1") {
		t.Fatalf("the old direct message is not read as before:\n%s", out)
	}
}

// readOutput is what the session of agent name in team has to read now: the messages the read returns, as the state
// held them before it advanced the session's cursor.
func readOutput(t *testing.T, b *Bus, name, team string) string {
	t.Helper()
	out := chk(t).ok(toolResult(server(b, name, inTeam(team)).read(0)))
	return out
}

func TestEachTeamReadsByItsOwnCursor(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	twoTeams(t, b)
	chk(t).ok(toolResult(server(b, "lead-a", inTeam("A")).send("codex-1", "take item 1.1 of team A")))
	chk(t).ok(toolResult(server(b, "lead-b", inTeam("B")).send("codex-1", "take item 1.2 of team B")))

	if out := readOutput(t, b, "codex-1", "A"); !strings.Contains(out, "take item 1.1 of team A") {
		t.Fatalf("the session of team A did not read its message:\n%s", out)
	}
	if out := readOutput(t, b, "codex-1", "B"); !strings.Contains(out, "take item 1.2 of team B") {
		t.Fatalf("reading in team A marked the message of team B read:\n%s", out)
	}
}

func TestAWokenMemberReadsTheMessagesOfItsTeamAndNotTheOldCursor(t *testing.T) {
	b := legacyBus(t)
	captureWakes(t)

	// the member has not read anything: it reads its lead's direct message and the team's notices, once, and its
	// cursor is kept for its team
	if out := readOutput(t, b, "member", "T"); !strings.Contains(out, "please take B1") {
		t.Fatalf("the member did not read the direct message of its lead:\n%s", out)
	}
	if out := readOutput(t, b, "member", "T"); !strings.Contains(out, "No new messages") {
		t.Fatalf("the member read its messages twice:\n%s", out)
	}
	mustWith(t, b, func(st *State) {
		if cur, ok := st.Cursors[messageCursor("member", "T")]; !ok || cur == 0 {
			t.Fatalf("the cursor of the member in its team was not kept: %v", st.Cursors)
		}
	})
}

func TestADirectMessageToANameOutsideMyTeamIsRefusedWithItsMembers(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	err := failure(t, SendMessageNotInYourTeam)(server(b, "lead-a", inTeam("A")).sendMessage("lead-b", "take item 1.1", true))

	if !strings.Contains(err.Message, "codex-1") || !strings.Contains(err.Message, "lead-a") {
		t.Fatalf("the refusal must list the members of the team, got: %s", err.Message)
	}
	if len(*woken) != 0 || strings.Contains(allMessages(t, b), "take item 1.1") {
		t.Fatalf("a refused message woke %v or was stored", *woken)
	}
}

func TestADirectMessageToTheHumanIsNotRefusedByATeamMember(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	twoTeams(t, b)

	chk(t).ok(toolResult(server(b, "lead-a", inTeam("A")).send("human", "the gate fails")))
}

func TestADirectMessageToTheTaskOfATeamMateIsDeliveredToIt(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	chk(t).ok(toolResult(server(b, "lead-a", inTeam("A")).send("task-codex-1-a", "take item 1.1")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-a" {
		t.Fatalf("woken %v, want the task named", got)
	}
	if out := readOutput(t, b, "codex-1", "A"); !strings.Contains(out, "take item 1.1") {
		t.Fatalf("the task named did not read its message:\n%s", out)
	}
}

func TestAMessageFromOutsideTeamsToAnAgentInTwoTeamsIsRefusedWithTheirTasks(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	err := failure(t, SendMessageAmbiguousAgent)(server(b, "human").sendMessage("codex-1", "please look", true))

	if !strings.Contains(err.Message, "task-codex-1-a") || !strings.Contains(err.Message, "task-codex-1-b") {
		t.Fatalf("the refusal must name every task, got: %s", err.Message)
	}
	if len(*woken) != 0 {
		t.Fatalf("a refused message woke %v", *woken)
	}
}

func TestAMessageFromOutsideTeamsToATaskIsDeliveredToThatTaskOnly(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)

	chk(t).ok(toolResult(server(b, "human").send("task-codex-1-b", "take item 1.2 of team B")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-b" {
		t.Fatalf("woken %v, want only the task named", got)
	}
	if out := readOutput(t, b, "codex-1", "B"); !strings.Contains(out, "take item 1.2 of team B") {
		t.Fatalf("the task named did not read its message:\n%s", out)
	}
	if out := readOutput(t, b, "codex-1", "A"); strings.Contains(out, "take item 1.2 of team B") {
		t.Fatalf("the other task read the message:\n%s", out)
	}
}

func TestAMessageFromOutsideTeamsToAnAgentInOneTeamIsBoundToIt(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "A", map[string][3]string{"lead-a": {"normal", "", "active"}, "codex-1": {"normal", "", "sleeping"}})

	chk(t).ok(toolResult(server(b, "human").send("codex-1", "please look")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1" {
		t.Fatalf("woken %v, want the agent's one task", got)
	}
	if out := readOutput(t, b, "codex-1", "A"); !strings.Contains(out, "please look") {
		t.Fatalf("the agent's team did not read the message:\n%s", out)
	}
}
