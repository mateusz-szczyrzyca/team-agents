package main

import (
	"testing"
	"time"
)

// TestLeadHeartbeatReturningNodeReportsBeforeExpiry keeps an outage-delayed start report on its original assignment.
func TestLeadHeartbeatReturningNodeReportsBeforeExpiry(t *testing.T) {
	b := newBus(t)
	old := Timestamp{time.Now().UTC().Add(-time.Hour)}
	var assigned *Job // original fencing evidence retained across the simulated outage
	storeRun(t, b, old, func(r *leadRegistry) {
		regSubmit(t, r, regRequest("returning"), old)
		assigned = regHeartbeat(t, r, regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal}), old)
		regSubmit(t, r, regRequest("silent"), old)
		regHeartbeat(t, r, regBeat("node-b", nil, FreeAgent{Name: "beta", Tier: TierNormal}), old)
	})
	if assigned == nil {
		t.Fatal("no original assignment")
	}
	a := &leadAPI{bus: b, ttl: time.Minute}
	beat := leadHeartbeatBody{
		NodeHeartbeat: regBeat("node-a", nil),
		Reports: []JobReport{{ID: "returning", Assignment: assigned.Assignment, State: JobRunning,
			TaskID: "local-task", Branch: "agents/node-a/job", At: Timestamp{old.Add(time.Second)}}},
	}
	// No sleep: the persisted last heartbeat is already older than the TTL.
	// Repeat the same report to verify recovery also preserves idempotency.
	for range 2 {
		if job, err := a.heartbeat("node-a", beat); err != nil {
			t.Fatalf("returning heartbeat: %v", err)
		} else if job != nil {
			t.Fatalf("started job offered for launch again: %+v", job)
		}
	}
	view, err := a.get("returning")
	if err != nil {
		t.Fatal(err)
	}
	if view.State != JobRunning || view.Node != "node-a" || view.Assignment != assigned.Assignment || view.NodeOffline || len(view.History) != 3 {
		t.Fatalf("returning assignment changed or expired: %+v", view)
	}
	nodes, nodesErr := a.listNodes()
	if nodesErr != nil {
		t.Fatal(nodesErr)
	}
	if len(nodes) != 2 || nodes[0].Node != "node-a" || nodes[0].Offline || !nodes[1].Offline {
		t.Fatalf("caller liveness or silent peer expiry: %+v", nodes)
	}
	silent, silentErr := a.get("silent")
	if silentErr != nil {
		t.Fatal(silentErr)
	}
	if silent.State != JobQueued {
		t.Fatalf("silent peer's unstarted job did not expire: %+v", silent)
	}
}
