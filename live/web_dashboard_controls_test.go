package main

import (
	"fmt"
	"net/http"
	"net/url"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// Rule: the compact dashboard selects the newest twenty tasks; full view also includes acknowledged history.
func TestDashboardCompactAndFullHistory(t *testing.T) {
	home := t.TempDir()
	for i := 0; i < 25; i++ {
		id := fmt.Sprintf("D%02d", i)
		created := time.Now().Add(time.Duration(i-25) * time.Hour).Format(time.RFC3339)
		addTask(t, home, id, "AGENT="+id+"\nSTATUS=stopped\nCREATED="+created+"\nFINISHED="+created+"\n")
	}
	writeFile(t, filepath.Join(home, "tasks", "D00", "acked"), "")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	if n := strings.Count(body, `href="/task/D`); n != 20 {
		t.Errorf("compact rows = %d, want 20", n)
	}
	if strings.Contains(body, `href="/task/D04"`) {
		t.Error("compact view retained an older task")
	}
	mustContain(t, "/", body, "Full dashboard view", `href="/dashboard/full"`)
	code, body := get(t, ts, "/dashboard/full")
	if code != http.StatusOK {
		t.Fatalf("full view status = %d", code)
	}
	if n := strings.Count(body, `href="/task/D`); n != 25 {
		t.Errorf("full rows = %d, want 25", n)
	}
	mustContain(t, "full", body, `sse-connect="/events/dashboard/full"`)
	for _, route := range []string{"/events/dashboard", "/events/dashboard/full"} {
		table := readEvent(t, openStream(t, ts, route), "table")
		want := 20
		if strings.HasSuffix(route, "/full") {
			want = 25
		}
		if n := strings.Count(table, `href="/task/D`); n != want {
			t.Errorf("%s rows = %d, want %d", route, n, want)
		}
	}
}

// Rule: stopping a local team member uses persistent stop, not the resumable kick operation.
func TestDashboardStopMemberPersistsIntent(t *testing.T) {
	home := fixture(t)
	var called []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) { called = args; return "stopped", nil })
	_, ts := testServer(t, home)
	code, _ := post(t, ts, "/task/T4/stop", url.Values{})
	if code != http.StatusOK || strings.Join(called, " ") != "stop T4" {
		t.Fatalf("stop member = %d %v", code, called)
	}
}

// Rule: team stop is an authenticated POST and validates the team before invoking the script.
func TestDashboardStopTeamAction(t *testing.T) {
	home := fixture(t)
	var calls []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		calls = append(calls, strings.Join(args, " "))
		return "stopped", nil
	})
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/teams")
	mustContain(t, "teams", body, `hx-post="/team/TT-team/stop"`, "Stop this team", "hx-confirm=")
	code, _ := post(t, ts, "/team/TT-team/stop", url.Values{})
	if code != http.StatusOK || len(calls) != 1 || calls[0] != "team TT-team --stop" {
		t.Fatalf("team stop = %d %v", code, calls)
	}
	code, _ = post(t, ts, "/team/missing/stop", url.Values{})
	if code != http.StatusNotFound || len(calls) != 1 {
		t.Fatalf("missing team = %d %v", code, calls)
	}
}

// Rule: persisted task stop wins over an obsolete bus state in both team displays.
func TestDashboardTeamDisplaysStoppedMember(t *testing.T) {
	home := fixture(t)
	addTask(t, home, "T5", "AGENT=member\nSTATUS=stopped\nTEAM=TT-team\n")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/teams")
	mustContain(t, "teams", body, "1 stopped")
	_, body = get(t, ts, "/team/TT-team")
	if strings.Contains(body, `class="st-dim">paused`) {
		t.Error("stopped member still shows the old bus state")
	}
	mustContain(t, "team", body, `class="st-stopped">stopped`)
}
