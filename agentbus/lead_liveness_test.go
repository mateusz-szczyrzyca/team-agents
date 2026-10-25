package main

import (
	"testing"
	"time"
)

// TestLeadHeartbeatWithPendingReportKeepsAssignment covers a node that returns after its TTL with a started job's
// report. The node is speaking, so its own heartbeat must refresh its liveness before the offline sweep runs:
// the report is accepted, the job stays with the node and moves to running, and the node is online.
func TestLeadHeartbeatWithPendingReportKeepsAssignment(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Nanosecond}
	if _, e := a.submit("node-a", true, regRequest("j")); e != nil {
		t.Fatal(e)
	}
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})}
	job, e := a.heartbeat("node-a", beat)
	if e != nil || job == nil {
		t.Fatalf("no assignment: job %v, error %v", job, e)
	}
	time.Sleep(time.Millisecond) // longer than the TTL: the node has been silent
	beat.Reports = []JobReport{{ID: "j", Assignment: job.Assignment, State: JobRunning, TaskID: "task", Branch: "branch"}}
	if _, e := a.heartbeat("node-a", beat); e != nil {
		t.Fatalf("report of a returning node rejected: %v", e)
	}
	a.ttl = time.Minute // read back with a normal liveness window
	view, ge := a.get("j")
	if ge != nil {
		t.Fatal(ge)
	}
	if view.State != JobRunning || view.NodeOffline {
		t.Fatalf("job state %s, node offline %v; want running on an online node", view.State, view.NodeOffline)
	}
}
