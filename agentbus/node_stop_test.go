package main

import (
	"encoding/json"
	"net/http"
	"testing"
	"time"
)

// assertHeartbeatStopDelivery exercises the round-six roadmap through HTTP and durable reloads.
func assertHeartbeatStopDelivery(t *testing.T) {
	t.Helper()
	b := newBus(t)
	a := &leadAPI{bus: b, ttl: time.Minute}
	req := regRequest("stop-delivery")
	if _, err := a.submit("node-a", true, req); err != nil {
		t.Fatal(err)
	}
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})}
	job, err := a.heartbeat("node-a", beat)
	if err != nil || job == nil {
		t.Fatalf("assignment=%+v, %v", job, err)
	}
	if _, err := a.report("node-a", req.ID, JobReport{Assignment: job.Assignment, State: JobRunning, TaskID: "local-task", Branch: "agents/alpha/result"}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.cancel(true, req.ID); err != nil {
		t.Fatal(err)
	}
	for retry := 0; retry < 3; retry++ {
		// Rebuild the handler to prove a stop is durable rather than a consumed in-memory event.
		h := newLeadHandler(b, "node-a", nil, time.Minute)
		w := httpLeadRequest(t, h, http.MethodPost, "/v1/heartbeat", beat)
		requireLeadStatus(t, w, http.StatusOK)
		var reply struct {
			Stops []leadJobView /* durable requests owned by this node */
		}
		if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
			t.Fatal(err)
		}
		if len(reply.Stops) != 1 || reply.Stops[0].Request.ID != req.ID || !reply.Stops[0].StopRequested {
			t.Fatalf("stop not redelivered: %s", w.Body.String())
		}
		other := httpLeadRequest(t, newLeadHandler(b, "node-b", nil, time.Minute), http.MethodPost, "/v1/heartbeat", leadHeartbeatBody{})
		var foreign struct {
			Stops []leadJobView /* requests must never cross node ownership */
		}
		if err := json.Unmarshal(other.Body.Bytes(), &foreign); err != nil {
			t.Fatal(err)
		}
		if len(foreign.Stops) != 0 {
			t.Fatal("stop sent to another node")
		}
	}
	if _, err := a.report("node-a", req.ID, JobReport{Assignment: job.Assignment, State: JobStopped, TaskID: "local-task", Branch: "agents/alpha/result"}); err != nil {
		t.Fatal(err)
	}
	w := httpLeadRequest(t, newLeadHandler(b, "node-a", nil, time.Minute), http.MethodPost, "/v1/heartbeat", beat)
	var reply struct {
		Stops []leadJobView /* acknowledgement removes the durable request */
	}
	if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
		t.Fatal(err)
	}
	if len(reply.Stops) != 0 {
		t.Fatal("acknowledged stop redelivered")
	}
}
