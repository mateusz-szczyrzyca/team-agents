package main

// Tests of the activity a node sends in its heartbeat (written first, docs/tdd.md): the lead keeps it only for the
// jobs assigned to that node, bounded, and stores it with the node's declaration in jobs.json; a heartbeat whose
// name differs from the socket peer is refused with both names.

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestHeartbeatKeepsActivityOfItsOwnJobsOnly(t *testing.T) {
	r := newLeadRegistry()
	regSubmit(t, r, regRequest("job-a"), regAt(0))
	h := regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})
	if got := regHeartbeat(t, r, h, regAt(1)); got == nil || got.Request.ID != "job-a" {
		t.Fatalf("job-a was not assigned to node-a: %+v", got)
	}
	lines := make([]string, 30)
	for i := range lines {
		lines[i] = fmt.Sprintf("line %d", i)
	}
	lines[29] = strings.Repeat("x", 1000)
	h.Activity = []JobActivity{
		{ID: "job-a", Agent: "alpha", Stage: "running", Lines: lines},
		{ID: "job-other", Agent: "alpha", Stage: "running", Lines: []string{"not this node's job"}},
	}
	regHeartbeat(t, r, h, regAt(2))
	got := r.nodes["node-a"].Heartbeat.Activity
	if len(got) != 1 || got[0].ID != "job-a" {
		t.Fatalf("activity kept: %+v, want job-a only", got)
	}
	if n := len(got[0].Lines); n != jobActivityLines {
		t.Fatalf("%d lines kept, want the latest %d", n, jobActivityLines)
	}
	if got[0].Lines[0] != "line 10" {
		t.Errorf("first kept line %q, want line 10 (the latest 20)", got[0].Lines[0])
	}
	if last := got[0].Lines[len(got[0].Lines)-1]; len([]rune(last)) > jobActivityLineRunes {
		t.Errorf("a line of %d runes kept, at most %d", len([]rune(last)), jobActivityLineRunes)
	}
}

func TestHeartbeatActivityIsStoredForTheWebInterface(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Minute}
	if _, e := a.submit("node-a", true, regRequest("j")); e != nil {
		t.Fatal(e)
	}
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})}
	if _, e := a.heartbeat("node-a", beat); e != nil {
		t.Fatal(e)
	}
	beat.Activity = []JobActivity{{ID: "j", Agent: "alpha", Stage: "running", Lines: []string{"10:00:05 checking the parser"}}}
	if _, e := a.heartbeat("node-a", beat); e != nil {
		t.Fatal(e)
	}
	data, err := os.ReadFile(filepath.Join(a.bus.dir, "jobs.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "checking the parser") {
		t.Errorf("jobs.json lacks the activity:\n%.400s", data)
	}
}

func TestHeartbeatIdentityMismatchNamesBoth(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Minute}
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("rpi5", nil)}
	_, e := a.heartbeat("rpi5-x", beat)
	failsWith(t, LeadHeartbeatUnauthorized)("mismatch", e)
	if e == nil || !strings.Contains(e.Message, `"rpi5"`) || !strings.Contains(e.Message, `"rpi5-x"`) {
		t.Errorf("the message should name both the declared and the socket peer's name: %v", e)
	}
}
