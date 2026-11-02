package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// mustNotContainSubstr asserts that unwanted substrings do not appear in body.
func mustNotContainSubstr(t *testing.T, where, body string, unwanted ...string) {
	t.Helper()
	for _, u := range unwanted {
		if strings.Contains(body, u) {
			t.Errorf("%s: unexpectedly contains %q", where, u)
		}
	}
}

// TestSyntheticTeamVisibleOnTeamsPage verifies that an investigation team record
// is listed on the /teams page and details page, while remaining excluded from the main dashboard (GET /).
func TestSyntheticTeamVisibleOnTeamsPage(t *testing.T) {
	home := fixture(t)
	now := time.Now()

	// Add synthetic investigation team task
	addTask(t, home, "T_INV_TEAM", "AGENT=investigation-team\nKIND=investigation_team\nSTATUS=running\nRUN=1\nTEAM=investigation-team\nCREATED="+now.Format(time.RFC3339)+"\n")

	// Update bus/state.json to include the team under st.Teams
	busState := `{
 "agents": {"solo": {"agent": "solo", "task": "T1", "status": "running the tests"}},
 "teams": {
   "investigation-team": {
     "id": "investigation-team", "lead": "investigation-team", "done": false,
     "members": {"investigation-team": {"state": "active", "tier": "normal", "task": "T_INV_TEAM"}},
     "board": {}
   }
 }
}`
	writeFile(t, filepath.Join(home, "bus", "state.json"), busState)

	_, ts := testServer(t, home)

	// Main dashboard (GET /) must NOT show synthetic investigation team row
	_, mainBody := get(t, ts, "/")
	mustNotContainSubstr(t, "/", mainBody, ">investigation-team<")

	// /teams page MUST list the team
	status, teamsBody := get(t, ts, "/teams")
	if status != 200 {
		t.Fatalf("/teams status = %d, want 200", status)
	}
	mustContain(t, "/teams", teamsBody, "investigation-team")

	// /team/{id} page MUST render the team details and its member task
	status, teamBody := get(t, ts, "/team/investigation-team")
	if status != 200 {
		t.Fatalf("/team/investigation-team status = %d, want 200", status)
	}
	mustContain(t, "/team/investigation-team", teamBody, "investigation-team", "T_INV_TEAM")
}

// TestRealAgentWithSimilarNamePreserved verifies that a real agent whose name is
// investigation-team-helper (or named investigation-team with KIND=worker) is not excluded from the dashboard.
func TestRealAgentWithSimilarNamePreserved(t *testing.T) {
	home := fixture(t)
	now := time.Now()

	addTask(t, home, "THELPER", "AGENT=investigation-team-helper\nKIND=worker\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")
	addTask(t, home, "TNAMED", "AGENT=investigation-team\nKIND=worker\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")

	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")

	mustContain(t, "/", body, ">investigation-team-helper<", ">investigation-team<")
}
