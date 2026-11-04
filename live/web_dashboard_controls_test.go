package main

import (
	"net/http"
	"net/url"
	"strings"
	"testing"
)

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
