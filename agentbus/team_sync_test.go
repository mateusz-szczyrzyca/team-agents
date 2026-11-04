package main

// Tests of team checkpoint isolation (written first, docs/tdd.md): once a team has an elected coordinator,
// a non-coordinator member's checkpoint call saves to that member's own task checkpoint file without overwriting
// the shared team checkpoint file (which --resume injects into every member's prompt) or broadcasting a team wake.

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestMemberCheckpointDoesNotPolluteTeamCheckpoint(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{
		"lead":   {"normal", "a", "active"},
		"member": {"normal", "b", "active"},
	})
	lead := server(b, "lead", inTeam("T"))
	member := server(b, "member", inTeam("T"))

	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.checkpoint("round 4 in progress", "fix review findings on PR #53")))
	chk(t).ok(toolResult(member.checkpoint("round 5 foundation laid", "start round 5 B10-B15")))

	teamData, err := os.ReadFile(filepath.Join(b.dir, "checkpoints", "team-T.md"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(teamData, []byte("round 4 in progress")) {
		t.Fatalf("team checkpoint should retain coordinator entry:\n%s", teamData)
	}
	if bytes.Contains(teamData, []byte("round 5 foundation laid")) {
		t.Fatalf("team checkpoint must not contain non-coordinator member entry:\n%s", teamData)
	}

	memberData, err := os.ReadFile(filepath.Join(b.dir, "checkpoints", "task-task-member.md"))
	if err != nil {
		t.Fatalf("member checkpoint should be written to its task checkpoint file: %v", err)
	}
	if !bytes.Contains(memberData, []byte("round 5 foundation laid")) {
		t.Fatalf("member task checkpoint should contain member entry:\n%s", memberData)
	}
}

// Rule: a member's own checkpoint says that it is archived rather than resumed and posts no team checkpoint event.
func TestMemberCheckpointIsTaskRecordWithoutBroadcast(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{
		"lead":   {"normal", "a", "active"},
		"member": {"normal", "b", "active"},
	})
	lead := server(b, "lead", inTeam("T"))
	member := server(b, "member", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.checkpoint("coordinator entry", "continue the round")))
	before, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	*woken = nil

	out := chk(t).ok(toolResult(member.checkpoint("member entry", "finish the assigned item")))
	if !strings.Contains(out, "not carried into a resume") || !strings.Contains(out, "agent archive") {
		t.Fatalf("member answer must explain the task record's lifecycle: %s", out)
	}
	after, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(after) != len(before) {
		t.Fatalf("member checkpoint broadcast a bus event: before %d messages, after %d", len(before), len(after))
	}
	if len(*woken) != 0 {
		t.Fatalf("member checkpoint woke tasks: %v", *woken)
	}
}

// Rule: before a coordinator is elected, a member's checkpoint remains the shared team checkpoint and is broadcast.
func TestMemberCheckpointBeforeCoordinatorUsesTeamRecord(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{
		"lead":   {"normal", "a", "active"},
		"member": {"normal", "b", "active"},
	})
	member := server(b, "member", inTeam("T"))
	chk(t).ok(toolResult(member.checkpoint("early team progress", "elect a coordinator")))

	teamData, err := os.ReadFile(filepath.Join(b.dir, "checkpoints", "team-T.md"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(teamData, []byte("early team progress")) {
		t.Fatalf("pre-coordinator checkpoint should be in the team record:\n%s", teamData)
	}
	if _, err := os.Stat(filepath.Join(b.dir, "checkpoints", "task-task-member.md")); !os.IsNotExist(err) {
		t.Fatalf("pre-coordinator checkpoint must not create a member task record: %v", err)
	}
	messages, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	found := false
	for _, message := range messages {
		if message.From == "member" && message.Kind == KindCheckpoint {
			found = true
		}
	}
	if !found {
		t.Fatal("pre-coordinator checkpoint should broadcast a checkpoint event")
	}
}
