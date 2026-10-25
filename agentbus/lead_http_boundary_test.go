package main

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// TestLeadHTTPReportsRespectAssignedNode checks that another authenticated instance cannot report a job.
func TestLeadHTTPReportsRespectAssignedNode(t *testing.T) {
	b := newBus(t)
	owner := newLeadHandler(b, "node-a", nil, time.Minute)
	other := newLeadHandler(b, "node-b", []string{"node-b"}, time.Minute)
	requireLeadStatus(t, httpLeadRequest(t, owner, http.MethodPost, "/v1/jobs", regRequest("j")), http.StatusOK)
	w := httpLeadRequest(t, owner, http.MethodPost, "/v1/heartbeat", leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})})
	requireLeadStatus(t, w, http.StatusOK)
	var reply struct{ Job *Job } // assigned job including fencing identity
	if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
		t.Fatal(err)
	}
	if reply.Job == nil {
		t.Fatal("no assignment")
	}
	report := JobReport{Assignment: reply.Job.Assignment, State: JobRunning, TaskID: "task", Branch: "branch"}
	w = httpLeadRequest(t, other, http.MethodPost, "/v1/jobs/j/state", report)
	requireLeadStatus(t, w, http.StatusForbidden)
	var failure struct{ Cause string } // stable cause for the authenticated wrong node
	if err := json.Unmarshal(w.Body.Bytes(), &failure); err != nil {
		t.Fatal(err)
	}
	if failure.Cause != "Unauthorized" {
		t.Fatalf("wrong node response: %s", w.Body.String())
	}
}

// TestLeadHTTPReadsExposeExpiryAndWaitingReason checks offline requeue, diagnostics, and public filtering.
func TestLeadHTTPReadsExposeExpiryAndWaitingReason(t *testing.T) {
	b := newBus(t)
	err := transactJobs(b, LeadSubmitStorage, regAt(0), func(r *leadRegistry) *LeadSubmitError {
		req := regRequest("j")
		req.Requires = []string{"hw:rp2350"}
		if _, e := r.submit(req, regAt(0)); e != nil {
			return e
		}
		_, he := r.heartbeat(regBeat("node-a", req.Requires, FreeAgent{Name: "alpha", Tier: TierNormal}), regAt(1))
		if he != nil {
			t.Fatal(he)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	h := newLeadHandler(b, "lead", nil, time.Second)
	w := httpLeadRequest(t, h, http.MethodGet, "/v1/jobs/j", nil)
	requireLeadStatus(t, w, http.StatusOK)
	var view leadJobView
	if e := json.Unmarshal(w.Body.Bytes(), &view); e != nil {
		t.Fatal(e)
	}
	if view.State != JobQueued || view.WaitReason != "no node with hw:rp2350" {
		t.Fatalf("expiry: %+v", view)
	}
	w = httpLeadRequest(t, h, http.MethodGet, "/v1/nodes", nil)
	requireLeadStatus(t, w, http.StatusOK)
	var nodes []leadNodeView
	if e := json.Unmarshal(w.Body.Bytes(), &nodes); e != nil {
		t.Fatal(e)
	}
	if len(nodes) != 1 || !nodes[0].Offline {
		t.Fatalf("offline node: %+v", nodes)
	}
	w = httpLeadRequest(t, h, http.MethodGet, "/v1/jobs?node=node-a", nil)
	requireLeadStatus(t, w, http.StatusOK)
	var jobs []Job
	if e := json.Unmarshal(w.Body.Bytes(), &jobs); e != nil {
		t.Fatal(e)
	}
	if len(jobs) != 0 {
		t.Fatal("requeued jobs still attributed to expired node")
	}
	for _, path := range []string{"/v1/jobs?unexpected=x", "/v1/jobs?node=a&node=b"} {
		requireLeadStatus(t, httpLeadRequest(t, h, http.MethodGet, path, nil), http.StatusBadRequest)
	}
	if e := os.WriteFile(filepath.Join(b.dir, "jobs.json"), []byte("broken"), 0o600); e != nil {
		t.Fatal(e)
	}
	w = httpLeadRequest(t, h, http.MethodGet, "/v1/jobs", nil)
	requireLeadStatus(t, w, http.StatusInternalServerError)
}
