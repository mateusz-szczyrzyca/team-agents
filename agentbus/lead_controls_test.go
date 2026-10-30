package main

import (
	_ "embed"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// jobsBeforeRound4 embeds a jobs.json format from before round 4.
//
//go:embed testdata/jobs-before-round4.json
var jobsBeforeRound4 []byte

// TestLeadJobsBeforeRound4Loads verifies that jobs.json from prior versions loads cleanly.
func TestLeadJobsBeforeRound4Loads(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "jobs.json"), jobsBeforeRound4, 0o600); err != nil {
		t.Fatal(err)
	}
	b := &Bus{dir: dir}
	reg, serr := b.loadJobs()
	if serr != nil {
		t.Fatalf("failed to load pre-round-4 jobs: %v", serr)
	}
	if len(reg.jobs) != 1 {
		t.Fatalf("expected 1 job, got %d", len(reg.jobs))
	}
	j := reg.jobs["attributed-job"]
	if j == nil || len(j.PendingAnswers) != 0 || len(j.PendingKicks) != 0 {
		t.Fatalf("unexpected pending answers/kicks on loaded legacy job: %+v", j)
	}
}

// TestLeadAnswerValidation checks flag, file name, size, controls and **ANSWERED** suffix checks.
func TestLeadAnswerValidation(t *testing.T) {
	_, err := parseLeadAnswerConfig([]string{"--node", "", "--task", "t1", "--file", "questions-1.md"})
	failsWith(t, LeadAnswerBadFlags)("missing node", err)

	_, err = parseLeadAnswerConfig([]string{"--node", "n1", "--task", "", "--file", "questions-1.md"})
	failsWith(t, LeadAnswerBadFlags)("missing task", err)

	_, err = parseLeadAnswerConfig([]string{"--node", "n1", "--task", "t1", "--file", ""})
	failsWith(t, LeadAnswerBadFlags)("missing file", err)

	_, err = parseLeadAnswerConfig([]string{"--node", "n1", "--task", "t1", "--file", "questions-0.md"})
	failsWith(t, LeadAnswerInvalidFile)("zero round", err)

	_, err = parseLeadAnswerConfig([]string{"--node", "n1", "--task", "t1", "--file", "questions.md"})
	failsWith(t, LeadAnswerInvalidFile)("no round number", err)

	a, _ := setupWaitingJob(t)

	// Oversized content
	huge := strings.Repeat("x", syncFileBytes+1) + "\n**ANSWERED**"
	_, err = a.answer("node-a", "t1", "questions-1.md", huge)
	failsWith(t, LeadAnswerContentTooLarge)("oversized content", err)

	// Missing **ANSWERED** suffix
	_, err = a.answer("node-a", "t1", "questions-1.md", "Verdict: ok\n")
	failsWith(t, LeadAnswerNotAnswered)("missing answered suffix", err)

	// Non-printable control characters
	_, err = a.answer("node-a", "t1", "questions-1.md", "Verdict: \x00\n**ANSWERED**")
	failsWith(t, LeadAnswerInvalidContent)("C0 control characters", err)
}

// TestLeadKickValidation checks flag validation for lead-kick.
func TestLeadKickValidation(t *testing.T) {
	_, err := parseLeadKickConfig([]string{"--node", "", "--task", "t1"})
	failsWith(t, LeadKickBadFlags)("missing node", err)

	_, err = parseLeadKickConfig([]string{"--node", "n1", "--task", ""})
	failsWith(t, LeadKickBadFlags)("missing task", err)

	_, err = parseLeadKickConfig([]string{"--node", "n!", "--task", "t1"})
	failsWith(t, LeadKickBadFlags)("invalid node identifier", err)
}

// setupWaitingJob creates a lead API with an assigned job in waiting state on node-a with taskID t1.
func setupWaitingJob(t *testing.T) (*leadAPI, string) {
	t.Helper()
	dir := t.TempDir()
	a := &leadAPI{bus: &Bus{dir: dir}, ttl: 90 * time.Second}
	jobReq := JobRequest{ID: "job-test-1", Repository: "repo", BaseRef: "main", Selector: "worker", Task: "task", Requester: "human"}
	sub, subErr := a.submit("lead", true, jobReq)
	if subErr != nil {
		t.Fatalf("submit: %v", subErr)
	}
	hb := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node:       "node-a",
			FreeAgents: []FreeAgent{{Name: "worker-1", Tier: TierWorker}},
		},
	}
	assigned, hbErr := a.heartbeat("node-a", hb)
	if hbErr != nil {
		t.Fatalf("heartbeat: %v", hbErr)
	}
	if assigned == nil || assigned.Request.ID != sub.Request.ID {
		t.Fatalf("expected assignment %s, got %v", sub.Request.ID, assigned)
	}
	// Report start (running)
	now := time.Now().UTC()
	rep := JobReport{
		ID:         sub.Request.ID,
		Node:       "node-a",
		Assignment: assigned.Assignment,
		State:      JobRunning,
		TaskID:     "t1",
		At:         Timestamp{now},
	}
	if _, repErr := a.report("node-a", sub.Request.ID, rep); repErr != nil {
		t.Fatalf("report running: %v", repErr)
	}
	// Report waiting
	rep.State = JobWaiting
	rep.At = Timestamp{now.Add(time.Second)}
	if _, repErr := a.report("node-a", sub.Request.ID, rep); repErr != nil {
		t.Fatalf("report waiting: %v", repErr)
	}
	return a, sub.Request.ID
}

// setupRunningJob helper sets up an active running job assigned to node-a with taskID t1 and mirror metadata.
func setupRunningJob(t *testing.T) (*leadAPI, string) {
	t.Helper()
	dir := t.TempDir()
	a := &leadAPI{bus: &Bus{dir: dir}, ttl: 90 * time.Second}
	jobReq := JobRequest{ID: "job-run-1", Repository: "repo", BaseRef: "main", Selector: "worker", Task: "task", Requester: "human"}
	sub, subErr := a.submit("lead", true, jobReq)
	if subErr != nil {
		t.Fatalf("submit: %v", subErr)
	}
	hb := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node:       "node-a",
			FreeAgents: []FreeAgent{{Name: "worker-1", Tier: TierWorker}},
		},
	}
	assigned, hbErr := a.heartbeat("node-a", hb)
	if hbErr != nil {
		t.Fatalf("heartbeat: %v", hbErr)
	}
	now := time.Now().UTC()
	rep := JobReport{
		ID:         sub.Request.ID,
		Node:       "node-a",
		Assignment: assigned.Assignment,
		State:      JobRunning,
		TaskID:     "t1",
		At:         Timestamp{now},
	}
	if _, repErr := a.report("node-a", sub.Request.ID, rep); repErr != nil {
		t.Fatalf("report running: %v", repErr)
	}
	syncBody := leadSyncBody{
		Job:        sub.Request.ID,
		Assignment: assigned.Assignment,
		TaskID:     "t1",
		Metadata: map[SyncMetadataKey]string{
			SyncStatus: "running",
			SyncRun:    "2",
		},
	}
	if err := a.sync("node-a", syncBody); err != nil {
		t.Fatalf("sync running metadata: %v", err)
	}
	return a, sub.Request.ID
}

// TestLeadAnswerSuccessAndIdempotency tests queueing answers, SHA-256 calculation, and idempotency.
func TestLeadAnswerSuccessAndIdempotency(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	content := "# Questions\n\n**Verdict:** a)\n\n**ANSWERED**\n"
	ans, aerr := a.answer("node-a", "t1", "questions-1.md", content)
	if aerr != nil {
		t.Fatalf("unexpected error queueing answer: %v", aerr)
	}
	if ans.Job != jobID || ans.TaskID != "t1" || ans.File != "questions-1.md" {
		t.Fatalf("unexpected answer fields: %+v", ans)
	}
	expectedSHA := sha256Hex(content)
	if ans.SHA256 != expectedSHA {
		t.Fatalf("expected SHA %s, got %s", expectedSHA, ans.SHA256)
	}

	// Idempotent re-queue
	ans2, aerr2 := a.answer("node-a", "t1", "questions-1.md", content)
	if aerr2 != nil || ans2.SHA256 != expectedSHA {
		t.Fatalf("re-queue failed or changed SHA: %v, %+v", aerr2, ans2)
	}

	// Non-existent job
	_, errNotFound := a.answer("node-a", "nonexistent-task", "questions-1.md", content)
	failsWith(t, LeadAnswerNoSuchJob)("no such task", errNotFound)
}

// TestLeadKickSuccessAndTerminalRejection tests queueing kicks, mirror run binding, and state checks.
func TestLeadKickSuccessAndTerminalRejection(t *testing.T) {
	// Kick on waiting job must be rejected
	aWait, _ := setupWaitingJob(t)
	_, errWait := aWait.kick("node-a", "t1")
	failsWith(t, LeadKickInvalidState)("waiting job kick", errWait)

	// Kick on running job succeeds and binds Run from mirror
	a, jobID := setupRunningJob(t)
	k, kerr := a.kick("node-a", "t1")
	if kerr != nil {
		t.Fatalf("unexpected error queueing kick: %v", kerr)
	}
	if k.Job != jobID || k.TaskID != "t1" || k.Run != 2 || k.SHA256 == "" {
		t.Fatalf("unexpected kick fields: %+v", k)
	}

	// Unknown task
	_, errUnknown := a.kick("node-a", "no-such-task")
	failsWith(t, LeadKickNoSuchJob)("unknown task", errUnknown)

	// Report done (terminal)
	repDone := JobReport{
		ID:         jobID,
		Node:       "node-a",
		Assignment: 1,
		State:      JobDone,
		TaskID:     "t1",
		At:         Timestamp{time.Now().UTC().Add(2 * time.Second)},
	}
	if _, repErr := a.report("node-a", jobID, repDone); repErr != nil {
		t.Fatalf("report done: %v", repErr)
	}

	// Kick on terminal job must be rejected
	_, errTerminal := a.kick("node-a", "t1")
	failsWith(t, LeadKickInvalidState)("terminal job kick", errTerminal)
}

// TestLeadHeartbeatDeliveryAndAcks verifies delivery in heartbeat and clearance upon ack.
func TestLeadHeartbeatDeliveryAndAcks(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	content := "Verdict: ok\n**ANSWERED**"
	ans := LeadAnswer{Job: jobID, TaskID: "t1", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)}
	kick := LeadKick{Job: jobID, TaskID: "t1", Run: 1, SHA256: sha256Hex(fmt.Sprintf("%s:t1:1", jobID))}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j := r.jobs[jobID]
		j.PendingAnswers = []LeadAnswer{ans}
		j.PendingKicks = []LeadKick{kick}
		r.jobs[jobID] = j
		return nil
	}); err != nil {
		t.Fatalf("setup pending: %v", err)
	}

	// Node sends heartbeat, should receive answers and kicks
	var deliveredAnswers []LeadAnswer
	var deliveredKicks []LeadKick
	hb := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node: "node-a",
		},
	}
	if _, herr := a.heartbeat("node-a", hb, &deliveredAnswers, &deliveredKicks); herr != nil {
		t.Fatalf("heartbeat: %v", herr)
	}
	if len(deliveredAnswers) != 1 || deliveredAnswers[0].SHA256 != ans.SHA256 {
		t.Fatalf("expected answer with SHA %s, got %+v", ans.SHA256, deliveredAnswers)
	}
	if len(deliveredKicks) != 1 || deliveredKicks[0].SHA256 != kick.SHA256 {
		t.Fatalf("expected kick with SHA %s, got %+v", kick.SHA256, deliveredKicks)
	}

	// Node acknowledges answer and kick in subsequent heartbeat
	hbWithAcks := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node: "node-a",
		},
		AnswerAcks: []LeadAnswerAck{{Job: jobID, TaskID: "t1", File: "questions-1.md", SHA256: ans.SHA256}},
		KickAcks:   []LeadKickAck{{Job: jobID, TaskID: "t1", Run: kick.Run, SHA256: kick.SHA256}},
	}
	var afterAcksAnswers []LeadAnswer
	var afterAcksKicks []LeadKick
	if _, herr := a.heartbeat("node-a", hbWithAcks, &afterAcksAnswers, &afterAcksKicks); herr != nil {
		t.Fatalf("heartbeat with acks: %v", herr)
	}
	if len(afterAcksAnswers) != 0 || len(afterAcksKicks) != 0 {
		t.Fatalf("expected empty answers and kicks after ack, got %d answers, %d kicks", len(afterAcksAnswers), len(afterAcksKicks))
	}
}

// TestLeadAnswerAndKickScopedAcks verifies identical content answers on different files or jobs are not cross-acked.
func TestLeadAnswerAndKickScopedAcks(t *testing.T) {
	a, job1ID := setupWaitingJob(t)
	// Create second waiting job on same node
	job2Req := JobRequest{ID: "job-test-2", Repository: "repo", BaseRef: "main", Selector: "worker", Task: "task2", Requester: "human"}
	if _, err := a.submit("lead", true, job2Req); err != nil {
		t.Fatalf("submit job2: %v", err)
	}
	hb := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{
			Node:       "node-a",
			FreeAgents: []FreeAgent{{Name: "worker-2", Tier: TierWorker}},
		},
	}
	assigned2, err := a.heartbeat("node-a", hb)
	if err != nil || assigned2 == nil {
		t.Fatalf("assign job2: %v", err)
	}
	now := time.Now().UTC()
	repRunning := JobReport{ID: job2Req.ID, Node: "node-a", Assignment: assigned2.Assignment, State: JobRunning, TaskID: "t2", At: Timestamp{now}}
	if _, err := a.report("node-a", job2Req.ID, repRunning); err != nil {
		t.Fatalf("report job2 running: %v", err)
	}
	repWaiting := JobReport{ID: job2Req.ID, Node: "node-a", Assignment: assigned2.Assignment, State: JobWaiting, TaskID: "t2", At: Timestamp{now.Add(time.Second)}}
	if _, err := a.report("node-a", job2Req.ID, repWaiting); err != nil {
		t.Fatalf("report job2 waiting: %v", err)
	}

	identicalContent := "Verdict: ok\n**ANSWERED**"
	sha := sha256Hex(identicalContent)
	ans1 := LeadAnswer{Job: job1ID, TaskID: "t1", File: "questions-1.md", Content: identicalContent, SHA256: sha}
	ans2 := LeadAnswer{Job: job2Req.ID, TaskID: "t2", File: "questions-1.md", Content: identicalContent, SHA256: sha}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j1 := r.jobs[job1ID]
		j1.PendingAnswers = []LeadAnswer{ans1}
		r.jobs[job1ID] = j1
		j2 := r.jobs[job2Req.ID]
		j2.PendingAnswers = []LeadAnswer{ans2}
		r.jobs[job2Req.ID] = j2
		return nil
	}); err != nil {
		t.Fatalf("setup pending: %v", err)
	}

	// Acking job1 should not clear job2
	hbAckJob1 := leadHeartbeatBody{
		NodeHeartbeat: NodeHeartbeat{Node: "node-a"},
		AnswerAcks:    []LeadAnswerAck{{Job: job1ID, TaskID: "t1", File: "questions-1.md", SHA256: ans1.SHA256}},
	}
	var deliveredAnswers []LeadAnswer
	if _, err := a.heartbeat("node-a", hbAckJob1, &deliveredAnswers); err != nil {
		t.Fatal(err)
	}
	if len(deliveredAnswers) != 1 || deliveredAnswers[0].Job != job2Req.ID {
		t.Fatalf("job2 answer was incorrectly cleared by job1 ack: %+v", deliveredAnswers)
	}
}

// TestLeadDoorbellIncludesAnswersAndKicks checks that pending answers and kicks update pendingFor.
func TestLeadDoorbellIncludesAnswersAndKicks(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	pendingBefore := a.pendingFor("node-a")
	if strings.Contains(pendingBefore, "a:") || strings.Contains(pendingBefore, "k:") {
		t.Fatalf("pending before queue should not have answers or kicks: %q", pendingBefore)
	}

	content := "Answer\n**ANSWERED**"
	ans := LeadAnswer{Job: jobID, TaskID: "t1", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j := r.jobs[jobID]
		j.PendingAnswers = []LeadAnswer{ans}
		r.jobs[jobID] = j
		return nil
	}); err != nil {
		t.Fatalf("setup pending answer: %v", err)
	}
	pendingAfterAnswer := a.pendingFor("node-a")
	if !strings.Contains(pendingAfterAnswer, "a:"+jobID) {
		t.Fatalf("pending after answer should contain a:%s, got %q", jobID, pendingAfterAnswer)
	}

	kick := LeadKick{Job: jobID, TaskID: "t1", Run: 1, SHA256: sha256Hex(fmt.Sprintf("%s:t1:1", jobID))}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j := r.jobs[jobID]
		j.PendingKicks = []LeadKick{kick}
		r.jobs[jobID] = j
		return nil
	}); err != nil {
		t.Fatalf("setup pending kick: %v", err)
	}
	pendingAfterKick := a.pendingFor("node-a")
	if !strings.Contains(pendingAfterKick, "k:"+jobID) {
		t.Fatalf("pending after kick should contain k:%s, got %q", jobID, pendingAfterKick)
	}
}

// TestLeadHeartbeatHTTPWireAnswersAndKicks checks JSON wire representation over HTTP handler.
func TestLeadHeartbeatHTTPWireAnswersAndKicks(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	content := "test answer\n**ANSWERED**"
	ans := LeadAnswer{Job: jobID, TaskID: "t1", File: "questions-1.md", Content: content, SHA256: sha256Hex(content)}
	kick := LeadKick{Job: jobID, TaskID: "t1", Run: 1, SHA256: sha256Hex(fmt.Sprintf("%s:t1:1", jobID))}
	if err := transactJobs(a.bus, LeadHeartbeatStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadHeartbeatError {
		j := r.jobs[jobID]
		j.PendingAnswers = []LeadAnswer{ans}
		j.PendingKicks = []LeadKick{kick}
		r.jobs[jobID] = j
		return nil
	}); err != nil {
		t.Fatalf("setup pending answers/kicks: %v", err)
	}

	handler := newLeadHandler(a.bus, "node-a", nil, time.Minute)
	reqBody := `{"Node":"node-a"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/heartbeat", strings.NewReader(reqBody))
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("HTTP heartbeat: %d %s", w.Code, w.Body.String())
	}
	bodyStr := w.Body.String()
	if !strings.Contains(bodyStr, ans.SHA256) || !strings.Contains(bodyStr, kick.SHA256) {
		t.Fatalf("heartbeat response wire missing answer SHA or kick SHA: %s", bodyStr)
	}

	// Ack via wire
	ackBody := fmt.Sprintf(`{"Node":"node-a","AnswerAcks":[{"Job":%q,"TaskID":"t1","File":"questions-1.md","SHA256":%q}],"KickAcks":[{"Job":%q,"TaskID":"t1","Run":%d,"SHA256":%q}]}`, jobID, ans.SHA256, jobID, kick.Run, kick.SHA256)
	reqAck := httptest.NewRequest(http.MethodPost, "/v1/heartbeat", strings.NewReader(ackBody))
	reqAck.RemoteAddr = "127.0.0.1:54321"
	wAck := httptest.NewRecorder()
	handler.ServeHTTP(wAck, reqAck)
	if wAck.Code != http.StatusOK {
		t.Fatalf("HTTP heartbeat ack: %d %s", wAck.Code, wAck.Body.String())
	}
	bodyAckStr := wAck.Body.String()
	if strings.Contains(bodyAckStr, ans.SHA256) || strings.Contains(bodyAckStr, kick.SHA256) {
		t.Fatalf("heartbeat response after ack should not contain answer SHA or kick SHA: %s", bodyAckStr)
	}
}

// TestLeadSyncQuestionsHook verifies that syncing a questions file invokes execQuestionsCmd.
func TestLeadSyncQuestionsHook(t *testing.T) {
	a, jobID := setupWaitingJob(t)
	a.questionsCmd = "/bin/dummy"
	called := ""
	orig := execQuestionsCmd
	execQuestionsCmd = func(_, target string) error {
		called = target
		return nil
	}
	defer func() { execQuestionsCmd = orig }()

	body := leadSyncBody{
		Job:        jobID,
		Assignment: 1,
		TaskID:     "t1",
		Files: []leadSyncFile{
			{Name: "questions-1.md", Content: "# Q\n", SHA256: sha256Hex("# Q\n")},
		},
	}
	if err := a.sync("node-a", body); err != nil {
		t.Fatalf("sync failed: %v", err)
	}
	if called != "t1@node-a" {
		t.Fatalf("expected questions hook called for t1@node-a, got %q", called)
	}

	called = ""
	bodyMeta := leadSyncBody{
		Job:        jobID,
		Assignment: 1,
		TaskID:     "t1",
		Metadata: map[SyncMetadataKey]string{
			SyncStatus: "waiting",
		},
	}
	if err := a.sync("node-a", bodyMeta); err != nil {
		t.Fatalf("sync metadata failed: %v", err)
	}
	if called != "t1@node-a" {
		t.Fatalf("expected questions hook called on waiting status for t1@node-a, got %q", called)
	}
}
