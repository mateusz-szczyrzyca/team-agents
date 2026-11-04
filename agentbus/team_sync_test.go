package main

// Tests of team checkpoint isolation (written first, docs/tdd.md): once a team has an elected coordinator,
// a non-coordinator member's checkpoint call saves to that member's own task checkpoint file without overwriting
// the shared team checkpoint file (which --resume injects into every member's prompt) or broadcasting a team wake.

import (
	"bytes"
	"os"
	"path/filepath"
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
