package main

import (
	"testing"
	"time"
)

// Tests of persisting the node's chosen agent on jobs (decision 6/7 gap, written first, docs/tdd.md):
// a report carries the actual agent who accepted a tier-selected job; the lead stores it on Job and
// leadJobView, rejects invalid agent identifiers, enforces immutability, and maps errors during heartbeats.

func TestReportPersistsChosenAgentWithoutActivity(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Minute}
	req := regRequest("job-tier")
	req.Selector = "worker"
	if _, err := a.submit("lead", true, req); err != nil {
		t.Fatalf("submit: %v", err)
	}
	hb := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node:       "node-a",
			FreeAgents: []FreeAgent{{Name: "worker-1", Tier: TierWorker}},
		},
	}
	assigned, herr := a.heartbeat("node-a", hb)
	if herr != nil || assigned == nil {
		t.Fatalf("heartbeat: herr=%v, assigned=%+v", herr, assigned)
	}

	runningRep := JobReport{
		ID:         "job-tier",
		Node:       "node-a",
		Assignment: assigned.Assignment,
		State:      JobRunning,
		TaskID:     "task-1",
		Agent:      "worker-1",
	}
	if _, rerr := a.report("node-a", "job-tier", runningRep); rerr != nil {
		t.Fatalf("report running: %v", rerr)
	}

	v1, gerr1 := a.get("job-tier")
	if gerr1 != nil {
		t.Fatalf("get running: %v", gerr1)
	}
	if v1.Agent != "worker-1" {
		t.Fatalf("running view.Agent = %q, want %q", v1.Agent, "worker-1")
	}

	doneRep := JobReport{
		ID:         "job-tier",
		Node:       "node-a",
		Assignment: assigned.Assignment,
		State:      JobDone,
		TaskID:     "task-1",
		Agent:      "worker-1",
		Result:     "success",
	}
	if _, derr := a.report("node-a", "job-tier", doneRep); derr != nil {
		t.Fatalf("report done: %v", derr)
	}

	v2, gerr2 := a.get("job-tier")
	if gerr2 != nil {
		t.Fatalf("get done: %v", gerr2)
	}
	if v2.Agent != "worker-1" {
		t.Fatalf("terminal view.Agent = %q, want %q", v2.Agent, "worker-1")
	}
}

func TestReportRejectsInvalidAgent(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-invalid-agent")
	req.Selector = "worker"
	r.submit(req, regAt(0))
	assigned, _ := r.heartbeat(NodeHeartbeat{Node: "node-a", FreeAgents: []FreeAgent{{Name: "worker-1", Tier: TierWorker}}}, regAt(1))
	if assigned == nil {
		t.Fatal("expected assignment")
	}

	for _, bad := range []string{"../escape", "worker/1", "has space", "worker\n1"} {
		rep := JobReport{
			ID:         "job-invalid-agent",
			Node:       "node-a",
			Assignment: assigned.Assignment,
			State:      JobRunning,
			Agent:      bad,
			At:         regAt(2),
		}
		_, err := r.report(rep, regAt(2))
		failsWith(t, LeadReportInvalidAgent)(bad, err)

		// Heartbeat report mapping must produce LeadHeartbeatInvalidAgent.
		hb := leadHeartbeatBody{
			NodeHeartbeat: NodeHeartbeat{Node: "node-a"},
			Reports:       []JobReport{rep},
		}
		a := &leadAPI{bus: newBus(t), ttl: time.Minute}
		// Register job in API bus.
		a.submit("lead", true, req)
		a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: NodeHeartbeat{Node: "node-a", FreeAgents: []FreeAgent{{Name: "worker-1", Tier: TierWorker}}}})
		_, hberr := a.heartbeat("node-a", hb)
		failsWith(t, LeadHeartbeatInvalidAgent)(bad, hberr)
	}
}

func TestReportAgentImmutable(t *testing.T) {
	r := newLeadRegistry()
	req := regRequest("job-immutable-agent")
	req.Selector = "worker"
	r.submit(req, regAt(0))
	assigned, _ := r.heartbeat(NodeHeartbeat{Node: "node-a", FreeAgents: []FreeAgent{{Name: "worker-1", Tier: TierWorker}}}, regAt(1))
	if assigned == nil {
		t.Fatal("expected assignment")
	}

	rep1 := JobReport{
		ID:         "job-immutable-agent",
		Node:       "node-a",
		Assignment: assigned.Assignment,
		State:      JobRunning,
		Agent:      "worker-1",
		At:         regAt(2),
	}
	if _, err := r.report(rep1, regAt(2)); err != nil {
		t.Fatalf("first report: %v", err)
	}

	rep2 := JobReport{
		ID:         "job-immutable-agent",
		Node:       "node-a",
		Assignment: assigned.Assignment,
		State:      JobRunning,
		Agent:      "worker-2",
		At:         regAt(3),
	}
	_, err2 := r.report(rep2, regAt(3))
	failsWith(t, LeadReportInvalidState)("mutated agent", err2)
}
