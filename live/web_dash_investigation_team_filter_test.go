package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// mustNotContain asserts that none of the unwanted substrings appear in the given body.
func mustNotContain(t *testing.T, where, body string, unwanted ...string) {
	t.Helper()
	for _, u := range unwanted {
		if strings.Contains(body, u) {
			t.Errorf("%s: unexpectedly contains %q", where, u)
		}
	}
}

// TestDashboardExcludesSyntheticInvestigationTeams verifies that synthetic investigation team rows
// (KIND=investigation_team, AGENT=investigation-team or investigator-team) are excluded from the main dashboard
// (both web dashboard GET / and panel snap.Tasks), while preserving real agents, real investigators, and snap.All.
func TestDashboardExcludesSyntheticInvestigationTeams(t *testing.T) {
	home := fixture(t)
	now := time.Now()

	// 1. Real agent worker task
	addTask(t, home, "TREAL", "AGENT=real-agent\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")

	// 2. Real investigator tasks
	addTask(t, home, "TINV1", "AGENT=claude-inv\nKIND=investigation\nINV_ID=10\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")
	addTask(t, home, "TINV2", "AGENT=codex-inv\nKIND=investigation\nINV_ID=11\nSTATUS=done\nRUN=1\nFINISHED="+now.Add(-5*time.Minute).Format(time.RFC3339)+"\n")

	// 3. Synthetic investigation team coordinator tasks (active and completed)
	addTask(t, home, "TTEAM1", "AGENT=investigation-team\nKIND=investigation_team\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")
	addTask(t, home, "TTEAM2", "AGENT=investigation-team\nKIND=investigation_team\nSTATUS=done\nRUN=1\nFINISHED="+now.Add(-5*time.Minute).Format(time.RFC3339)+"\n")
	addTask(t, home, "TTEAM3", "AGENT=investigator-team\nKIND=investigation_team\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")

	_, ts := testServer(t, home)

	// Verify main dashboard (GET /)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, ">real-agent<", "investigator #10 (claude-inv)", "investigator #11 (codex-inv)")
	mustNotContain(t, "/", body, "investigation-team", "investigator-team")

	// Verify Snapshot: snap.Tasks (used by terminal live panel) must exclude synthetic investigation teams
	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	for _, task := range snap.Tasks {
		if task.Kind == "investigation_team" || task.Agent == "investigation-team" || task.Agent == "investigator-team" {
			t.Errorf("snap.Tasks contains synthetic investigation team task: id=%s agent=%s kind=%s", task.ID, task.Agent, task.Kind)
		}
	}

	// Verify snap.All still retains the synthetic team task for lookup and /teams integration
	if snap.All["TTEAM1"] == nil {
		t.Errorf("snap.All missing TTEAM1")
	}

	// Verify /teams endpoint renders cleanly
	status, teamsBody := get(t, ts, "/teams")
	if status != 200 {
		t.Errorf("/teams returned status %d", status)
	}
	mustContain(t, "/teams", teamsBody, "Teams")
}
