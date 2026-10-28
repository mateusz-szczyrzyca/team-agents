package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// syncFixture creates an accepted start without running any external commands.
func syncFixture(t *testing.T) (*leadAPI, leadSyncBody) {
	t.Helper()
	b := newBus(t)
	a := &leadAPI{bus: b, ttl: time.Minute}
	_, err := a.submit("node-a", true, regRequest("mirror-job"))
	if err != nil {
		t.Fatal(err)
	}
	job, herr := a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil, FreeAgent{Name: "alpha", Tier: TierNormal})})
	if herr != nil || job == nil {
		t.Fatalf("assign: %v", herr)
	}
	_, rerr := a.report("node-a", job.Request.ID, JobReport{Assignment: job.Assignment, State: JobRunning, TaskID: "task-a"})
	if rerr != nil {
		t.Fatal(rerr)
	}
	return a, leadSyncBody{Job: job.Request.ID, Assignment: job.Assignment, TaskID: "task-a", Metadata: map[SyncMetadataKey]string{"STATUS": "interrupted", "RESTARTS": "2", "JOB_ID": job.Request.ID}, Files: []leadSyncFile{syncFile("final.md", "finished\n")}}
}

// syncFile computes the digest of the exact wire content.
func syncFile(name SyncFilename, content string) leadSyncFile {
	sum := sha256.Sum256([]byte(content))
	return leadSyncFile{Name: string(name), Content: content, SHA256: hex.EncodeToString(sum[:])}
}

// TestLeadSyncValidation requires a distinct typed cause for each rejected boundary.
func TestLeadSyncValidation(t *testing.T) {
	cases := []struct {
		name   string
		node   string
		change func(*leadSyncBody)
		cause  LeadSyncCause
	}{
		{"node", "../escape", func(*leadSyncBody) {}, LeadSyncInvalidNode},
		{"task", "node-a", func(b *leadSyncBody) { b.TaskID = "../escape" }, LeadSyncInvalidTask},
		{"other-task", "node-a", func(b *leadSyncBody) { b.TaskID = "task-b" }, LeadSyncInvalidTask},
		{"owner", "node-b", func(*leadSyncBody) {}, LeadSyncUnauthorized},
		{"missing", "node-a", func(b *leadSyncBody) { b.Job = "absent" }, LeadSyncNoSuchJob},
		{"fence", "node-a", func(b *leadSyncBody) { b.Assignment++ }, LeadSyncStaleAssignment},
		{"key", "node-a", func(b *leadSyncBody) { b.Metadata["WORKTREE"] = "/tmp" }, LeadSyncInvalidMetadata},
		{"job-key", "node-a", func(b *leadSyncBody) { b.Metadata["JOB_ID"] = "other" }, LeadSyncInvalidMetadata},
		{"filename", "node-a", func(b *leadSyncBody) { b.Files[0] = syncFile("../final.md", "x") }, LeadSyncInvalidFile},
		{"duplicate", "node-a", func(b *leadSyncBody) { b.Files = append(b.Files, b.Files[0]) }, LeadSyncInvalidFile},
		{"file-bytes", "node-a", func(b *leadSyncBody) { b.Files[0] = syncFile("final.md", strings.Repeat("x", 256*1024+1)) }, LeadSyncFileTooLarge},
		{"activity-lines", "node-a", func(b *leadSyncBody) { b.Files[0] = syncFile("activity.jsonl", strings.Repeat("{}\n", 2001)) }, LeadSyncFileTooLarge},
		{"error-lines", "node-a", func(b *leadSyncBody) { b.Files[0] = syncFile("err-1.log", strings.Repeat("x\n", 501)) }, LeadSyncFileTooLarge},
		{"digest", "node-a", func(b *leadSyncBody) { b.Files[0].SHA256 = strings.Repeat("0", 64) }, LeadSyncInvalidDigest},
		{"logs-count", "node-a", func(b *leadSyncBody) { b.Logs = make([]json.RawMessage, 501) }, LeadSyncInvalidLogs},
		{"logs-scope", "node-a", func(b *leadSyncBody) {
			b.Logs = []json.RawMessage{json.RawMessage(`{"level":"info","task":"another"}`)}
		}, LeadSyncInvalidLogs},
		{"logs-size", "node-a", func(b *leadSyncBody) {
			b.Logs = []json.RawMessage{json.RawMessage(`{"level":"warn","message":"` + strings.Repeat("x", 65536) + `"}`)}
		}, LeadSyncInvalidLogs},
	}
	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			a, b := syncFixture(t)
			tt.change(&b)
			err := a.sync(tt.node, b)
			var typed *LeadSyncError
			if err == nil || !errors.As(err.asError(), &typed) || typed.Cause != tt.cause {
				t.Fatalf("cause got %v want %v", err, tt.cause)
			}
			if _, err := os.Stat(filepath.Join(a.bus.dir, "remote")); !os.IsNotExist(err) {
				t.Fatalf("rejected sync wrote mirror: %v", err)
			}
		})
	}
}

// TestLeadSyncStoresIncrementalArtifacts verifies restarts, cut markers and file preservation.
func TestLeadSyncStoresIncrementalArtifacts(t *testing.T) {
	a, b := syncFixture(t)
	b.Files = append(b.Files, syncFile("activity.jsonl", strings.Repeat("{}\n", 2000)), syncFile("err-1.log", strings.Repeat("x\n", 500)), syncFile("prompt-1.md", strings.Repeat("x", 256*1024)), syncFile("questions-1.md", "question"), syncFile("result.md", "result"))
	b.Files[2].Cut = true
	b.Logs = []json.RawMessage{json.RawMessage(`{"ts":"2026-10-28T12:00:00Z","level":"warn","event":"node_warn"}`), json.RawMessage(`{"level":"info","task":"task-a","event":"task_info"}`)}
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(a.bus.dir, "remote", "node-a", "task-a")
	data, err := os.ReadFile(filepath.Join(dir, "final.md")) // #nosec G304 -- dir is built from newBus(t) and fixed test identifiers.
	if err != nil || string(data) != "finished\n" {
		t.Fatalf("file: %q %v", data, err)
	}
	meta, err := os.ReadFile(filepath.Join(dir, "metadata.json")) // #nosec G304 -- dir is built from newBus(t) and fixed test identifiers.
	if err != nil || !strings.Contains(string(meta), `"RESTARTS":"2"`) {
		t.Fatalf("metadata: %q %v", meta, err)
	}
	manifest, err := os.ReadFile(filepath.Join(dir, "sync.json")) // #nosec G304 -- dir is built from newBus(t) and fixed test identifiers.
	if err != nil || !strings.Contains(string(manifest), `"Cut":true`) {
		t.Fatalf("cut manifest: %q %v", manifest, err)
	}
	// Lost acknowledgements may retry after lead restart without duplicating logs.
	a = &leadAPI{bus: &Bus{dir: a.bus.dir}, ttl: time.Minute}
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	logs, err := os.ReadFile(filepath.Join(a.bus.dir, "remote", "node-a", "logs.jsonl"))
	if err != nil || strings.Count(string(logs), "\n") != 2 {
		t.Fatalf("logs retry: %q %v", logs, err)
	}
	b.Metadata = map[SyncMetadataKey]string{"STATUS": "done"}
	b.Files = []leadSyncFile{syncFile("final.md", "new result")}
	b.Logs = nil
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	meta, err = os.ReadFile(filepath.Join(dir, "metadata.json")) // #nosec G304 -- dir is built from newBus(t) and fixed test identifiers.
	if err != nil || !strings.Contains(string(meta), `"RESTARTS":"2"`) || !strings.Contains(string(meta), `"STATUS":"done"`) {
		t.Fatalf("metadata merge: %q %v", meta, err)
	}
	data, err = os.ReadFile(filepath.Join(dir, "prompt-1.md")) // #nosec G304 -- dir is built from newBus(t) and fixed test identifiers.
	if err != nil || len(data) != 256*1024 {
		t.Fatalf("unchanged prompt lost: %d %v", len(data), err)
	}
}

// TestLeadSyncRetentionAndLogCap ties mirrors to jobs and bounds the node log history.
func TestLeadSyncRetentionAndLogCap(t *testing.T) {
	a, b := syncFixture(t)
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(a.bus.dir, "remote", "node-a", "logs.jsonl")
	if err := os.WriteFile(path, []byte(strings.Repeat("{}\n", 20000)), 0600); err != nil {
		t.Fatal(err)
	}
	b.Logs = []json.RawMessage{json.RawMessage(`{"level":"error"}`)}
	if err := a.sync("node-a", b); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path) // #nosec G304 -- path is inside the temporary bus with fixed node identifiers.
	if err != nil || strings.Count(string(data), "\n") != 20000 || !strings.HasSuffix(string(data), "{\"level\":\"error\"}\n") {
		t.Fatalf("log cap: bytes=%d %v", len(data), err)
	}
	old := Timestamp{time.Now().Add(-31 * 24 * time.Hour)}
	storeRun(t, a.bus, regAt(0), func(r *leadRegistry) {
		j := r.jobs[b.Job]
		j.State = JobDone
		for i := range j.History {
			j.History[i].At = Timestamp{old.Add(-time.Duration(len(j.History)-i) * time.Second)}
		}
		j.History = append(j.History, JobTransition{State: JobDone, At: old})
	})
	if _, err := a.listJobs(""); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(a.bus.dir, "remote", "node-a", "task-a")); !os.IsNotExist(err) {
		t.Fatalf("expired mirror retained: %v", err)
	}
}

// TestLeadSyncEnvelopeAndStorage distinguishes total envelope and filesystem failures.
func TestLeadSyncEnvelopeAndStorage(t *testing.T) {
	a, b := syncFixture(t)
	b.Files = nil
	for i := 0; i < 9; i++ {
		b.Files = append(b.Files, syncFile(SyncFilename("prompt-"+strconv.Itoa(i+1)+".md"), strings.Repeat("x", 256*1024)))
	}
	failsWith(t, LeadSyncRequestTooLarge)("encoded envelope", a.sync("node-a", b))
	a, b = syncFixture(t)
	if err := os.WriteFile(filepath.Join(a.bus.dir, "remote"), []byte("blocked"), 0600); err != nil {
		t.Fatal(err)
	}
	failsWith(t, LeadSyncStorage)("mirror storage", a.sync("node-a", b))
}

// TestLeadSyncHTTP enforces peer/browser authentication, encoded request size and strict JSON.
func TestLeadSyncHTTP(t *testing.T) {
	a, b := syncFixture(t)
	b.Files = append(b.Files, syncFile("prompt-1.md", strings.Repeat("x", 256*1024)))
	h := newLeadHandler(a.bus, "node-a", nil, time.Minute)
	w := httpLeadRequest(t, h, http.MethodPost, "/v1/sync", b)
	requireLeadStatus(t, w, http.StatusOK)
	for _, tt := range []struct {
		name, body, header, cause string
		status                    int
	}{
		{"too-large", `{"x":"` + strings.Repeat("x", 2*1024*1024) + `"}`, "", "RequestTooLarge", 400},
		{"unknown", `{"Unknown":true}`, "", "MalformedBody", 400},
		{"trailing", `{} {}`, "", "MalformedBody", 400},
		{"browser", `{}`, "Origin", "Unauthorized", 403},
	} {
		t.Run(tt.name, func(t *testing.T) {
			r := httptest.NewRequest(http.MethodPost, "/v1/sync", strings.NewReader(tt.body))
			r.RemoteAddr = "127.0.0.1:2"
			if tt.header != "" {
				r.Header.Set(tt.header, "https://example.com")
			}
			w := httptest.NewRecorder()
			h.ServeHTTP(w, r)
			if w.Code != tt.status || !strings.Contains(w.Body.String(), `"Cause":"`+tt.cause+`"`) {
				t.Fatalf("HTTP %d %.300s", w.Code, w.Body.String())
			}
		})
	}
	// The node client accepts the dedicated envelope but keeps other routes bounded.
	cfg, err := parseNodeRequest([]string{"--url", "http://127.0.0.1:1", "--path", "/v1/sync"})
	if err != nil || cfg.path != "/v1/sync" {
		t.Fatalf("sync client route: %v", err)
	}
}
