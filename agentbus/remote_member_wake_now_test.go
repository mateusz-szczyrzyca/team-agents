package main

import (
	"strings"
	"testing"
)

// A request to resume a remote member's paused task now (agent wake on its proxy task) waits in its node's heartbeat
// replies — and rings its doorbell — while the member is paused, and is done once the member is reported at work.
func TestWakeNowWaitsForTheNodeUntilTheMemberRuns(t *testing.T) {
	_, a, _, _ := remoteTeam(t)
	if e := a.wakeMemberNow("solo@node2"); e != nil {
		t.Fatal(e)
	}
	if got := a.wakesNowFor("node2"); len(got) != 1 || got[0] != "job-m" {
		t.Fatalf("wakes now %q, want [job-m]", got)
	}
	if p := a.pendingFor("node2"); !strings.Contains(p, "job-m") {
		t.Errorf("the doorbell's %q does not carry the request", p)
	}
	if _, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "paused"}}); err != nil {
		t.Fatal(err)
	}
	if got := a.wakesNowFor("node2"); len(got) != 1 {
		t.Errorf("still paused: the request stays, got %q", got)
	}
	if _, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "running"}}); err != nil {
		t.Fatal(err)
	}
	if got := a.wakesNowFor("node2"); len(got) != 0 {
		t.Errorf("at work again: the request is done, got %q", got)
	}
	failsWith(t, LeadMemberNoSuchMember)("unknown", a.wakeMemberNow("nobody@node2"))
}
