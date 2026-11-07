package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestDeriveGitHubParams_InvestigationOriginalTeam verifies peer color registration for investigations.
func TestDeriveGitHubParams_InvestigationOriginalTeam(t *testing.T) {
	tmp := t.TempDir()
	for _, sub := range []string{"tasks/inv1", "tasks/peer1"} {
		if err := os.MkdirAll(filepath.Join(tmp, sub), 0o750); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(tmp, "tasks/inv1/meta.env"), []byte("AGENT=inv1\nTEAM=t1\nKIND=investigation\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(tmp, "tasks/peer1/meta.env"), []byte("AGENT=peer1\nTEAM=t1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg := newConfig()
	cfg.Scalars["AGENT_GITHUB_COLORS"] = "#112233 #445566"
	params := GitHubRunDerivationParams{
		AgentsHome: tmp, TasksDir: filepath.Join(tmp, "tasks"), BusDir: filepath.Join(tmp, "bus"),
		RunnerID: "inv1", Config: cfg,
	}
	p, err := DeriveGitHubParams(context.Background(), params)
	if err != nil || p.Team != "" || !strings.Contains(p.GitHubNameColors, "peer1=#") {
		t.Fatalf("unexpected params: err=%v team=%q colors=%q", err, p.Team, p.GitHubNameColors)
	}
}
