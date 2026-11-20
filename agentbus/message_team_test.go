package main

// A direct message is for the named agent in the team of its sender: it wakes only that team's task and is read only by
// that team's session. An agent may be in two open teams at once, with a task and a worktree in each; nothing sent in one
// of them may reach the other (the incident of 19.11.2026: the lead of a second team wrote to an agent, the session of the
// first team woke up and hard-reset the branch of the second team's task).

import (
	_ "embed"
	"os"
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
