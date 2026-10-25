package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// httpLeadRequest exercises the real handler with a trusted local socket peer, without external programs.
func httpLeadRequest(t *testing.T, h http.Handler, method, path string, body any) *httptest.ResponseRecorder {
	t.Helper()
	data, err := json.Marshal(body)
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRequest(method, path, bytes.NewReader(data))
	r.RemoteAddr = "127.0.0.1:1234"
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	return w
}

// requireLeadStatus checks the transport contract and includes only a bounded diagnostic on failure.
func requireLeadStatus(t *testing.T, w *httptest.ResponseRecorder, status int) {
	t.Helper()
	if w.Code != status {
		t.Fatalf("HTTP %d, want %d: %.500s", w.Code, status, w.Body.String())
	}
	if w.Header().Get("Content-Type") != "application/json" {
		t.Fatal("response is not JSON")
	}
}

// TestLeadHTTPPublicLifecycle verifies durable submit, assignment, reports, views and cancellation through HTTP.
func TestLeadHTTPPublicLifecycle(t *testing.T) {
	b := newBus(t)
	h := newLeadHandler(b, "node-a", nil, time.Minute)
	req := regRequest("http-job")
	w := httpLeadRequest(t, h, http.MethodPost, "/v1/jobs", req)
	requireLeadStatus(t, w, http.StatusOK)
	var job Job
	if err := json.Unmarshal(w.Body.Bytes(), &job); err != nil {
		t.Fatal(err)
	}
	if job.State != JobQueued || job.Request.ID != req.ID {
		t.Fatalf("submit: %+v", job)
	}
	// A new handler on the same directory proves the HTTP adapter uses durable storage.
	h = newLeadHandler(b, "node-a", nil, time.Minute)
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})}
	w = httpLeadRequest(t, h, http.MethodPost, "/v1/heartbeat", beat)
	requireLeadStatus(t, w, http.StatusOK)
	var reply struct{ Job *Job } // assignment returned by a successful heartbeat
	if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
		t.Fatal(err)
	}
	if reply.Job == nil || reply.Job.State != JobAssigned {
		t.Fatalf("heartbeat: %s", w.Body.String())
	}
	job = *reply.Job
	at := Timestamp{time.Now().Add(time.Second).UTC().Truncate(time.Second).Add(123456789 * time.Nanosecond)}
	rep := JobReport{ID: req.ID, Assignment: job.Assignment, State: JobRunning, TaskID: "local-task", Branch: "agents/node-a/job", At: at}
	data, err := json.Marshal(rep)
	if err != nil {
		t.Fatal(err)
	}
	// Ordinary Timestamp encoding is intentionally legacy; network callers send precise RFC3339Nano.
	var raw map[string]any
	if err := json.Unmarshal(data, &raw); err != nil {
		t.Fatal(err)
	}
	raw["At"] = at.UTC().Format(time.RFC3339Nano)
	w = httpLeadRequest(t, h, http.MethodPost, "/v1/jobs/http-job/state", raw)
	requireLeadStatus(t, w, http.StatusOK)
	w = httpLeadRequest(t, h, http.MethodGet, "/v1/jobs/http-job", nil)
	requireLeadStatus(t, w, http.StatusOK)
	if err := json.Unmarshal(w.Body.Bytes(), &job); err != nil {
		t.Fatal(err)
	}
	if job.State != JobRunning || !job.History[len(job.History)-1].At.Equal(at.Time) {
		t.Fatalf("report precision lost: %+v", job)
	}
	if strings.Contains(w.Body.String(), `"Seq"`) || strings.Contains(w.Body.String(), "NextAssignment") {
		t.Fatal("internal registry counters exposed")
	}
	w = httpLeadRequest(t, h, http.MethodPost, "/v1/jobs/http-job/cancel", nil)
	requireLeadStatus(t, w, http.StatusOK)
	if err := json.Unmarshal(w.Body.Bytes(), &job); err != nil {
		t.Fatal(err)
	}
	if !job.StopRequested || job.State != JobRunning {
		t.Fatalf("started cancellation: %+v", job)
	}
	rep.State, rep.Result, rep.At = JobDone, "finished", Timestamp{at.Add(time.Second)}
	beat.Reports = []JobReport{rep}
	w = httpLeadRequest(t, h, http.MethodPost, "/v1/heartbeat", beat)
	requireLeadStatus(t, w, http.StatusOK)
	w = httpLeadRequest(t, h, http.MethodGet, "/v1/jobs", nil)
	requireLeadStatus(t, w, http.StatusOK)
	var jobs []Job
	if err := json.Unmarshal(w.Body.Bytes(), &jobs); err != nil {
		t.Fatal(err)
	}
	if len(jobs) != 1 || jobs[0].State != JobDone || jobs[0].Result != "finished" {
		t.Fatalf("jobs: %+v", jobs)
	}
	w = httpLeadRequest(t, h, http.MethodGet, "/v1/nodes", nil)
	requireLeadStatus(t, w, http.StatusOK)
	var nodes []leadNodeView
	if err := json.Unmarshal(w.Body.Bytes(), &nodes); err != nil {
		t.Fatal(err)
	}
	if len(nodes) != 1 || nodes[0].Node != "node-a" || nodes[0].Offline {
		t.Fatalf("nodes: %+v", nodes)
	}
	if _, err := os.Stat(filepath.Join(b.dir, "state.json")); !os.IsNotExist(err) {
		t.Fatalf("ordinary bus state touched: %v", err)
	}
}

// TestLeadHTTPRejectsBadBodies verifies bounded, single-object JSON with operation-specific causes.
func TestLeadHTTPRejectsBadBodies(t *testing.T) {
	h := newLeadHandler(newBus(t), "node-a", nil, time.Minute)
	for _, path := range []string{"/v1/jobs", "/v1/heartbeat", "/v1/jobs/j/state"} {
		for _, body := range []string{`null`, `{`, `{} {}`, `{"unknown":true}`, strings.Repeat(" ", 300000) + `{}`} {
			r := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
			r.RemoteAddr = "127.0.0.1:1234"
			w := httptest.NewRecorder()
			h.ServeHTTP(w, r)
			requireLeadStatus(t, w, http.StatusBadRequest)
			var failure struct{ Cause string } // stable operation cause, not a reconstructed Go error
			if err := json.Unmarshal(w.Body.Bytes(), &failure); err != nil {
				t.Fatal(err)
			}
			if failure.Cause != "MalformedBody" {
				t.Fatalf("%s: %s", path, w.Body.String())
			}
		}
	}
}

// TestLeadHTTPAuthorizationNeverTrustsForwardedHeaders covers authorization separately for each operation.
func TestLeadHTTPAuthorizationNeverTrustsForwardedHeaders(t *testing.T) {
	h := newLeadHandler(newBus(t), "node-a", nil, time.Minute)
	for _, endpoint := range []struct {
		method string // operation's HTTP verb
		path   string // operation's route
	}{
		{http.MethodPost, "/v1/heartbeat"}, {http.MethodPost, "/v1/jobs"},
		{http.MethodPost, "/v1/jobs/j/state"}, {http.MethodPost, "/v1/jobs/j/cancel"},
		{http.MethodGet, "/v1/jobs"}, {http.MethodGet, "/v1/jobs/j"}, {http.MethodGet, "/v1/nodes"},
	} {
		r := httptest.NewRequest(endpoint.method, endpoint.path, strings.NewReader(`{}`))
		r.RemoteAddr = "invalid-socket-address"
		r.Header.Set("X-Forwarded-For", "127.0.0.1")
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		requireLeadStatus(t, w, http.StatusForbidden)
		var failure struct{ Cause string } // stable wire cause
		if err := json.Unmarshal(w.Body.Bytes(), &failure); err != nil {
			t.Fatal(err)
		}
		if failure.Cause != "Unauthorized" {
			t.Fatalf("%s: %s", endpoint.path, w.Body.String())
		}
	}
}

// TestLeadAPISubmitCauses verifies typed validation and identity binding at the durable adapter.
func TestLeadAPISubmitCauses(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Minute}
	tests := []struct {
		name  string            // rule under test
		edit  func(*JobRequest) // invalid submission mutation
		cause LeadSubmitCause   // exact typed failure
	}{
		{"missing", func(r *JobRequest) { r.Task = "" }, LeadSubmitMissingArgument},
		{"id", func(r *JobRequest) { r.ID = "../x" }, LeadSubmitInvalidID},
		{"base", func(r *JobRequest) { r.BaseRef = "--help" }, LeadSubmitInvalidBase},
		{"caps", func(r *JobRequest) { r.Requires = []string{"sign:key"} }, LeadSubmitInvalidCapabilities},
		{"size", func(r *JobRequest) { r.Task = strings.Repeat("x", 65537) }, LeadSubmitTaskTooLarge},
		{"repo", func(r *JobRequest) { r.Repository = "/tmp/repo" }, LeadSubmitMalformedBody},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			r := regRequest("j")
			tc.edit(&r)
			_, e := a.submit("node-a", true, r)
			failsWith(t, tc.cause)(tc.name, e)
		})
	}
	r := regRequest("j")
	_, e := a.submit("node-a", false, r)
	failsWith(t, LeadSubmitUnauthorized)("remote human", e)
	r.Requester = "alpha@node-b"
	r.BaseSHA = strings.Repeat("a", 40)
	_, e = a.submit("node-a", false, r)
	failsWith(t, LeadSubmitUnauthorized)("spoofed requester", e)
	r.Requester = "alpha@node-a"
	if _, e = a.submit("node-a", false, r); e != nil {
		t.Fatal(e)
	}
	r.Task = "different"
	_, e = a.submit("node-a", false, r)
	failsWith(t, LeadSubmitJobConflict)("conflict", e)
}

// TestLeadAPIReportAndHeartbeatCauses verifies ownership, fencing, lifecycle and rollback across both adapters.
func TestLeadAPIReportAndHeartbeatCauses(t *testing.T) {
	a := &leadAPI{bus: newBus(t), ttl: time.Minute}
	if _, e := a.submit("node-a", true, regRequest("j")); e != nil {
		t.Fatal(e)
	}
	beat := leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})}
	job, e := a.heartbeat("node-a", beat)
	if e != nil {
		t.Fatal(e)
	}
	if job == nil {
		t.Fatal("no assignment")
	}
	rep := JobReport{ID: "j", Assignment: job.Assignment, State: JobRunning, TaskID: "task", Branch: "branch"}
	_, re := a.report("node-b", "j", rep)
	failsWith(t, LeadReportUnauthorized)("other node", re)
	rep.Assignment++
	_, re = a.report("node-a", "j", rep)
	failsWith(t, LeadReportStaleAssignment)("fence", re)
	rep.Assignment = job.Assignment
	rep.ID = "other"
	_, re = a.report("node-a", "j", rep)
	failsWith(t, LeadReportMalformedBody)("path mismatch", re)
	rep.ID = "j"
	rep.Node = "node-b"
	_, re = a.report("node-a", "j", rep)
	failsWith(t, LeadReportUnauthorized)("body spoof", re)
	rep.Node = ""
	rep.State = JobQueued
	_, re = a.report("node-a", "j", rep)
	failsWith(t, LeadReportInvalidState)("bad transition", re)
	rep.State = JobRunning
	beat.Reports = []JobReport{rep, {ID: "missing", State: JobDone}}
	_, e = a.heartbeat("node-a", beat)
	failsWith(t, LeadHeartbeatNoSuchJob)("batch rollback", e)
	got, ge := a.get("j")
	if ge != nil {
		t.Fatal(ge)
	}
	if got.State != JobAssigned {
		t.Fatal("failed batch saved partial progress")
	}
	beat.Reports = []JobReport{rep}
	beat.Capabilities = []string{"sign:key"}
	_, e = a.heartbeat("node-a", beat)
	failsWith(t, LeadHeartbeatInvalidCapabilities)("caps", e)
	beat.Capabilities = nil
	beat.Node = "node-b"
	_, e = a.heartbeat("node-a", beat)
	failsWith(t, LeadHeartbeatUnauthorized)("node spoof", e)
	beat.Node = "node-a"
	beat.Reports[0].Assignment++
	_, e = a.heartbeat("node-a", beat)
	failsWith(t, LeadHeartbeatStaleAssignment)("heartbeat fence", e)
	beat.Reports[0] = rep
	beat.Reports[0].State = JobQueued
	_, e = a.heartbeat("node-a", beat)
	failsWith(t, LeadHeartbeatInvalidState)("heartbeat transition", e)
	beat.Reports[0] = rep
	if _, e = a.heartbeat("node-a", beat); e != nil {
		t.Fatal(e)
	}
	rep.State = JobDone
	rep.Result = "ok"
	if _, re = a.report("node-a", "j", rep); re != nil {
		t.Fatal(re)
	}
	rep.Result = "changed"
	_, re = a.report("node-a", "j", rep)
	failsWith(t, LeadReportTerminalConflict)("terminal conflict", re)
	beat.Reports = []JobReport{rep}
	_, e = a.heartbeat("node-a", beat)
	failsWith(t, LeadHeartbeatTerminalConflict)("heartbeat conflict", e)
	_, re = a.report("node-a", "missing", JobReport{})
	failsWith(t, LeadReportNoSuchJob)("missing", re)
	_, ce := a.cancel(false, "j")
	failsWith(t, LeadCancelUnauthorized)("remote cancel", ce)
	_, ce = a.cancel(true, "missing")
	failsWith(t, LeadCancelNoSuchJob)("missing cancel", ce)
	_, ge = a.get("missing")
	failsWith(t, LeadGetJobNoSuchJob)("missing get", ge)
	_, le := a.listJobs("../node")
	failsWith(t, LeadListJobsInvalidArgument)("invalid filter", le)
}

// TestLeadAPIStorageCauses ensures every operation preserves its own typed storage cause.
func TestLeadAPIStorageCauses(t *testing.T) {
	b := newBus(t)
	if e := os.WriteFile(filepath.Join(b.dir, "jobs.json"), []byte("broken"), 0o600); e != nil {
		t.Fatal(e)
	}
	a := &leadAPI{bus: b, ttl: time.Minute}
	_, se := a.submit("node-a", true, regRequest("j"))
	failsWith(t, LeadSubmitStorage)("submit storage", se)
	_, he := a.heartbeat("node-a", leadHeartbeatBody{})
	failsWith(t, LeadHeartbeatStorage)("heartbeat storage", he)
	_, re := a.report("node-a", "j", JobReport{})
	failsWith(t, LeadReportStorage)("report storage", re)
	_, ce := a.cancel(true, "j")
	failsWith(t, LeadCancelStorage)("cancel storage", ce)
	_, ge := a.get("j")
	failsWith(t, LeadGetJobStorage)("get storage", ge)
	_, le := a.listJobs("")
	failsWith(t, LeadListJobsStorage)("list storage", le)
	_, ne := a.listNodes()
	failsWith(t, LeadListNodesStorage)("nodes storage", ne)
}

// leadConcurrentRequests verifies that simultaneous HTTP submissions survive fresh storage transactions.
func leadConcurrentRequests(t *testing.T) {
	t.Helper()
	h := newLeadHandler(newBus(t), "node-a", nil, time.Minute)
	var wg sync.WaitGroup // joins the independent HTTP callers
	for i := range 20 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			w := httpLeadRequest(t, h, http.MethodPost, "/v1/jobs", regRequest(fmt.Sprintf("j%d", i)))
			if w.Code != http.StatusOK {
				t.Errorf("submit: %d", w.Code)
			}
		}()
	}
	wg.Wait()
	w := httpLeadRequest(t, h, http.MethodGet, "/v1/jobs", nil)
	requireLeadStatus(t, w, http.StatusOK)
	var jobs []Job
	if e := json.Unmarshal(w.Body.Bytes(), &jobs); e != nil {
		t.Fatal(e)
	}
	if len(jobs) != 20 {
		t.Fatalf("lost submissions: %d", len(jobs))
	}
}
