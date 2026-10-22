package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// fakeLeases is a registry with one instance holding one lease generation.
type fakeLeases struct {
	instance   string // the instance that holds the lease
	generation int64  // the current lease generation
}

// Live reports whether instance holds the lease.
func (f fakeLeases) Live(instance string) bool { return instance == f.instance }

// Current reports whether generation is the current lease of instance.
func (f fakeLeases) Current(instance string, generation int64) bool {
	return f.Live(instance) && generation == f.generation
}

// commandHub returns a hub whose authenticated peer is instance "mac" with the given leases and poll window.
func commandHub(t *testing.T, leases CommandLeases, poll time.Duration) *Hub {
	t.Helper()
	return &Hub{
		Bus:          newBus(t),
		Authenticate: func(*http.Request) (string, *HubAuthError) { return "mac", nil },
		Leases:       leases,
		PollDuration: poll,
	}
}

// postCommand sends a raw body to a command route through the hub's HTTP handler.
func postCommand(t *testing.T, h *Hub, method, path, body string) *httptest.ResponseRecorder {
	t.Helper()
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest(method, path, strings.NewReader(body)))
	return w
}

// decodeReply decodes a command-channel reply body.
func decodeReply(t *testing.T, w *httptest.ResponseRecorder) commandReply {
	t.Helper()
	var reply commandReply
	if err := json.Unmarshal(w.Body.Bytes(), &reply); err != nil {
		t.Fatalf("reply %q: %v", w.Body.String(), err)
	}
	return reply
}

// pollBody encodes a poll request for lease generation 1, the generation the tests start with, from the start of the journal.
func pollBody(t *testing.T) string {
	t.Helper()
	data, err := json.Marshal(commandPollRequest{Version: hubProtocolVersion, Generation: 1, After: 0})
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// ackBody encodes an acknowledgement for the given lease generation.
func ackBody(t *testing.T, generation, seq int64, outcome CommandOutcome) string {
	t.Helper()
	data, err := json.Marshal(commandAckRequest{Version: hubProtocolVersion, Generation: generation, Seq: seq, Outcome: outcome})
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// wireCause checks that a reply carries the named cause of op and that the name is a known cause.
func wireCause(t *testing.T, reply commandReply, op, cause string) {
	t.Helper()
	if reply.Error == nil {
		t.Fatalf("expected an error %s/%s, got %+v", op, cause, reply)
	}
	if reply.Error.Op != op || reply.Error.Cause != cause {
		t.Fatalf("error = %s/%s, want %s/%s (%s)", reply.Error.Op, reply.Error.Cause, op, cause, reply.Error.Message)
	}
}

func TestCommandEnqueueRejectsUnknownKind(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	failsWith(t, CommandEnqueueUnknownKind)("enqueue", h.EnqueueCommand("mac", "reboot", "t1", nil))
}

func TestCommandEnqueueRejectsMissingInstanceOrTask(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	failsWith(t, CommandEnqueueInvalidParams)("enqueue", h.EnqueueCommand("", CommandStopTask, "t1", map[string]string{"reason": "x"}))
	failsWith(t, CommandEnqueueInvalidParams)("enqueue", h.EnqueueCommand("mac", CommandStopTask, "", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueRejectsMissingRequiredParameter(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	failsWith(t, CommandEnqueueInvalidParams)("enqueue", h.EnqueueCommand("mac", CommandStartRun, "t1", map[string]string{"agent": "claude"}))
}

func TestCommandEnqueueFailsWhenInstanceIsUnreachable(t *testing.T) {
	failsWith(t, CommandEnqueueInstanceUnreachable)("enqueue",
		commandHub(t, nil, 0).EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
	failsWith(t, CommandEnqueueInstanceUnreachable)("enqueue",
		commandHub(t, fakeLeases{"other", 1}, 0).EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueFailsWhenQueueIsFull(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	for i := 0; i < commandQueueLimit; i++ {
		if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}); err != nil {
			t.Fatalf("enqueue %d: %v", i, err)
		}
	}
	failsWith(t, CommandEnqueueQueueFull)("enqueue", h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueFailsOnStorageError(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	// A directory where the journal belongs cannot be read as a file.
	if err := os.Mkdir(filepath.Join(h.Bus.dir, commandStoreFile), 0o700); err != nil {
		t.Fatal(err)
	}
	failsWith(t, CommandEnqueueStorage)("enqueue", h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueLogsTheQueuedCommand(t *testing.T) {
	logs := captureLogs(t)
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := h.EnqueueCommand("mac", CommandWake, "t1", map[string]string{"member": "codex-1", "reason": "message"}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(logs.String(), `"event":"command_queued"`) {
		t.Fatalf("no command_queued record: %s", logs.String())
	}
}

func TestCommandPollDeliversQueuedCommandsInOrder(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := h.EnqueueCommand("mac", CommandStartRun, "t1", map[string]string{"agent": "claude-1", "run": "1"}); err != nil {
		t.Fatal(err)
	}
	if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "done"}); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t))
	reply := decodeReply(t, w)
	if w.Code != http.StatusOK || len(reply.Commands) != 2 {
		t.Fatalf("poll: status %d, %+v", w.Code, reply)
	}
	if reply.Commands[0].Seq != 1 || reply.Commands[1].Seq != 2 || reply.Commands[0].Kind != CommandStartRun ||
		reply.Commands[0].Params["agent"] != "claude-1" || reply.Commands[1].Kind != CommandStopTask {
		t.Fatalf("commands out of order or wrong: %+v", reply.Commands)
	}
}

func TestCommandPollReturnsNoContentWhenWindowEnds(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 50*time.Millisecond)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t))
	if w.Code != http.StatusNoContent || w.Body.Len() != 0 {
		t.Fatalf("empty poll: status %d body %q", w.Code, w.Body.String())
	}
}

func TestCommandPollWakesWhenCommandArrives(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 5*time.Second)
	go func() {
		time.Sleep(100 * time.Millisecond)
		if err := h.EnqueueCommand("mac", CommandStopTask, "t9", map[string]string{"reason": "stop"}); err != nil {
			t.Error("enqueue in test goroutine:", err)
		}
	}()
	start := time.Now()
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t))
	reply := decodeReply(t, w)
	if w.Code != http.StatusOK || len(reply.Commands) != 1 || reply.Commands[0].Task != "t9" {
		t.Fatalf("long poll: status %d %+v", w.Code, reply)
	}
	if time.Since(start) >= 5*time.Second {
		t.Fatal("the poll waited for the whole window instead of returning the new command")
	}
}

func TestCommandPollRejectsStaleLease(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 2}, 0)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t))
	reply := decodeReply(t, w)
	if w.Code != http.StatusConflict {
		t.Fatalf("stale lease: status %d", w.Code)
	}
	wireCause(t, reply, "command_poll", "LeaseStale")
	if cause, ok := CommandPollCauseByName(reply.Error.Cause); !ok || cause != CommandPollLeaseStale {
		t.Fatalf("cause name does not rebuild the typed cause: %v %v", cause, ok)
	}
}

func TestCommandPollFailsClosedWithoutRegistry(t *testing.T) {
	h := commandHub(t, nil, 0)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t))
	if w.Code != http.StatusConflict {
		t.Fatalf("no registry: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "LeaseStale")
}

func TestCommandPollRejectsMalformedRequests(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	for name, body := range map[string]string{
		"not JSON":      "{",
		"unknown field": `{"version":1,"generation":1,"after":0,"extra":true}`,
		"two objects":   pollBody(t) + pollBody(t),
		"wrong version": `{"version":99,"generation":1,"after":0}`,
	} {
		w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", body)
		if w.Code != http.StatusBadRequest {
			t.Fatalf("%s: status %d", name, w.Code)
		}
		wireCause(t, decodeReply(t, w), "command_poll", "Malformed")
	}
}

func TestCommandRoutesRejectOtherMethodsAndPaths(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if w := postCommand(t, h, http.MethodGet, "/v1/commands/poll", ""); w.Code != http.StatusBadRequest {
		t.Fatalf("GET: status %d", w.Code)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/nope", "{}")
	if w.Code != http.StatusNotFound {
		t.Fatalf("unknown route: status %d", w.Code)
	}
	reply := decodeReply(t, w)
	if reply.Error == nil || reply.Error.Cause != "UnknownOperation" {
		t.Fatalf("unknown route reply: %+v", reply)
	}
}

func TestCommandsSurviveHubRestart(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := h.EnqueueCommand("mac", CommandWake, "t1", map[string]string{"member": "codex-1", "reason": "board"}); err != nil {
		t.Fatal(err)
	}
	restarted := &Hub{Bus: h.Bus, Authenticate: h.Authenticate, Leases: fakeLeases{"mac", 1}, PollDuration: 0}
	reply := decodeReply(t, postCommand(t, restarted, http.MethodPost, "/v1/commands/poll", pollBody(t)))
	if len(reply.Commands) != 1 || reply.Commands[0].Kind != CommandWake {
		t.Fatalf("command lost across restart: %+v", reply)
	}
}

func TestCommandPollFailsOnStorageError(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 50*time.Millisecond)
	if err := os.Mkdir(filepath.Join(h.Bus.dir, commandStoreFile), 0o700); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t))
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("storage failure: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "Storage")
}

func TestCommandAckRemovesCommandAndRecordsOutcome(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 50*time.Millisecond)
	if err := h.EnqueueCommand("mac", CommandStartRun, "t1", map[string]string{"agent": "claude-1", "run": "3"}); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, 1, 1, CommandOutcomeStarted))
	if w.Code != http.StatusOK {
		t.Fatalf("ack: status %d %s", w.Code, w.Body.String())
	}
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t)); w.Code != http.StatusNoContent {
		t.Fatalf("acknowledged command was delivered again: status %d", w.Code)
	}
	store, serr := h.Bus.loadCommandStore()
	if serr != nil {
		t.Fatal(serr)
	}
	results := store.Queues["mac"].Results
	if len(results) != 1 || results[0].Seq != 1 || results[0].Outcome != CommandOutcomeStarted || results[0].Kind != CommandStartRun {
		t.Fatalf("result not recorded: %+v", results)
	}
}

func TestCommandAckKeepsOnlyRecentResults(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	for i := 0; i < commandResultLimit+5; i++ {
		if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}); err != nil {
			t.Fatal(err)
		}
		if w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, 1, int64(i+1), CommandOutcomeStopped)); w.Code != http.StatusOK {
			t.Fatalf("ack %d: status %d", i, w.Code)
		}
	}
	store, serr := h.Bus.loadCommandStore()
	if serr != nil {
		t.Fatal(serr)
	}
	results := store.Queues["mac"].Results
	if len(results) != commandResultLimit || results[0].Seq != 6 {
		t.Fatalf("results kept: %d, first %d", len(results), results[0].Seq)
	}
}

func TestCommandAckErrors(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}); err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name   string
		body   string
		status int
		cause  string
	}{
		{"unknown sequence number", ackBody(t, 1, 99, CommandOutcomeStopped), http.StatusNotFound, "NoSuchCommand"},
		{"unknown outcome", ackBody(t, 1, 1, "exploded"), http.StatusBadRequest, "InvalidOutcome"},
		{"stale lease", ackBody(t, 7, 1, CommandOutcomeStopped), http.StatusConflict, "LeaseStale"},
		{"malformed body", "not json", http.StatusBadRequest, "Malformed"},
	}
	for _, c := range cases {
		w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", c.body)
		if w.Code != c.status {
			t.Fatalf("%s: status %d, want %d", c.name, w.Code, c.status)
		}
		wireCause(t, decodeReply(t, w), "command_ack", c.cause)
	}
	// The command is still pending after every failed acknowledgement.
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t)); w.Code != http.StatusOK {
		t.Fatalf("pending command lost after failed acks: status %d", w.Code)
	}
}

func TestCommandAckRejectsSecondAcknowledgement(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}); err != nil {
		t.Fatal(err)
	}
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, 1, 1, CommandOutcomeStopped)); w.Code != http.StatusOK {
		t.Fatalf("first ack: status %d", w.Code)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, 1, 1, CommandOutcomeStopped))
	wireCause(t, decodeReply(t, w), "command_ack", "NoSuchCommand")
}

func TestCommandAckFailsOnStorageError(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := os.Mkdir(filepath.Join(h.Bus.dir, commandStoreFile), 0o700); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, 1, 1, CommandOutcomeStopped))
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("storage failure: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_ack", "Storage")
}

func TestCommandJournalKeepsItsFormatVersion(t *testing.T) {
	h := commandHub(t, fakeLeases{"mac", 1}, 0)
	if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(h.Bus.dir, commandStoreFile))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(data, []byte(`"version": 1`)) {
		t.Fatalf("journal without a version: %s", data)
	}
	// A journal written by a newer protocol version is refused, not reinterpreted.
	if err := os.WriteFile(filepath.Join(h.Bus.dir, commandStoreFile), []byte(`{"version":2,"queues":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, serr := h.Bus.loadCommandStore(); serr == nil {
		t.Fatal("a journal of another version was accepted")
	}
}

func TestCommandKindAndOutcomeVocabularies(t *testing.T) {
	var kind CommandKind
	if err := json.Unmarshal([]byte(`"start_run"`), &kind); err != nil || kind != CommandStartRun {
		t.Fatalf("known kind: %v %q", err, kind)
	}
	if err := json.Unmarshal([]byte(`"reboot_host"`), &kind); err == nil {
		t.Fatal("unknown kind accepted")
	}
	for _, o := range []CommandOutcome{CommandOutcomeStarted, CommandOutcomeAlreadyRunning, CommandOutcomeStopped,
		CommandOutcomeUnknownTask, CommandOutcomeDelivered, CommandOutcomeFailed} {
		if !o.Valid() {
			t.Fatalf("outcome %q rejected", o)
		}
	}
	if CommandOutcome("failed:disk").Valid() {
		t.Fatal("the reason must be the detail field, not part of the outcome")
	}
}
