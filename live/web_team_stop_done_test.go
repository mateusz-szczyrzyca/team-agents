package main

import (
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// finishTeam marks the fixture's team TT-team as closed in the bus state, as finish_team does.
func finishTeam(t *testing.T, home string) {
	t.Helper()
	path := filepath.Join(home, "bus", "state.json")
	data, err := fs.ReadFile(os.DirFS(filepath.Dir(path)), filepath.Base(path))
	if err != nil {
		t.Fatal(err)
	}
	closed := strings.Replace(string(data), `"done": false`, `"done": true`, 1)
	if closed == string(data) {
		t.Fatal("the fixture's team has no done flag to set")
	}
	writeFile(t, path, closed)
}

// Rule: a finished team offers no "Stop this team" — there is nothing left to stop — on either team display.
func TestFinishedTeamOffersNoStop(t *testing.T) {
	home := fixture(t)
	finishTeam(t, home)
	_, ts := testServer(t, home)
	for _, route := range []string{"/teams", "/team/TT-team"} {
		_, body := get(t, ts, route)
		mustContain(t, route, body, "TT-team")
		if strings.Contains(body, "Stop this team") || strings.Contains(body, `hx-post="/team/TT-team/stop"`) {
			t.Errorf("%s offers to stop a finished team", route)
		}
	}
}

// Rule: a team stopped by the human still says so on /teams once it is closed.
func TestStoppedFinishedTeamSaysStopped(t *testing.T) {
	home := fixture(t)
	finishTeam(t, home)
	addTask(t, home, "T4", "AGENT=lead\nSTATUS=stopped\nRUN=1\nTEAM=TT-team\nTEAM_STOPPED=1\n")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/teams")
	mustContain(t, "/teams", body, `<span class="dim">stopped</span>`)
	if strings.Contains(body, "Stop this team") {
		t.Error("/teams offers to stop a stopped team")
	}
}
