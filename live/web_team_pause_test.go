package main

import (
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Rule: a working team shows "Pause this team" on the list and on its page; the POST pauses it through the script
// (agent pause <team>), after the team is validated.
func TestDashboardPauseTeamAction(t *testing.T) {
	home := fixture(t)
	var calls []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		calls = append(calls, strings.Join(args, " "))
		return "paused", nil
	})
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/teams")
	mustContain(t, "teams", body, `hx-post="/team/TT-team/pause"`, "Pause this team")
	_, page := get(t, ts, "/team/TT-team")
	mustContain(t, "team page", page, `hx-post="/team/TT-team/pause"`, "Pause this team")
	if strings.Contains(page, "/team/TT-team/unpause") {
		t.Fatalf("a working team offers unpause")
	}
	code, _ := post(t, ts, "/team/TT-team/pause", url.Values{})
	if code != http.StatusOK || len(calls) != 1 || calls[0] != "pause TT-team" {
		t.Fatalf("team pause = %d %v", code, calls)
	}
	code, _ = post(t, ts, "/team/missing/pause", url.Values{})
	if code != http.StatusNotFound || len(calls) != 1 {
		t.Fatalf("missing team = %d %v", code, calls)
	}
}

// Rule: a paused team (AGENTS_HOME/paused/<team>) shows "paused" and "Unpause this team" instead, and the POST
// unpauses it through the script (agent unpause <team>).
func TestDashboardUnpauseTeamAction(t *testing.T) {
	home := fixture(t)
	if err := os.MkdirAll(filepath.Join(home, "paused"), 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, "paused", "TT-team"), []byte("since=1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	var calls []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		calls = append(calls, strings.Join(args, " "))
		return "unpaused", nil
	})
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/teams")
	mustContain(t, "teams", body, `hx-post="/team/TT-team/unpause"`, "Unpause this team", "paused")
	if strings.Contains(body, `hx-post="/team/TT-team/pause"`) {
		t.Fatalf("a paused team offers pause")
	}
	_, page := get(t, ts, "/team/TT-team")
	mustContain(t, "team page", page, `hx-post="/team/TT-team/unpause"`, "Unpause this team")
	code, _ := post(t, ts, "/team/TT-team/unpause", url.Values{})
	if code != http.StatusOK || len(calls) != 1 || calls[0] != "unpause TT-team" {
		t.Fatalf("team unpause = %d %v", code, calls)
	}
	code, _ = post(t, ts, "/team/missing/unpause", url.Values{})
	if code != http.StatusNotFound || len(calls) != 1 {
		t.Fatalf("missing team = %d %v", code, calls)
	}
}
