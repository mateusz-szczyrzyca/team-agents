package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

// commandFixture is a hub state directory with one instance "mac" registered by the peer "token:mac".
type commandFixture struct {
	bus      *Bus                 // the hub's state directory, shared by the registry and the command journal
	registry *HubInstanceRegistry // the lease registry that validates peers, tokens and generations
	lease    HubInstanceLease     // the lease issued to the owning node
}

// newCommandFixture registers instance "mac" for the peer "token:mac" in a fresh state directory.
func newCommandFixture(t *testing.T) *commandFixture {
	t.Helper()
	bus := newBus(t)
	registry := &HubInstanceRegistry{Bus: bus}
	lease, err := registry.Register("mac", "token:mac", instanceRegisterFixture("mac", "session-one"))
	if err != nil {
		t.Fatalf("register: %v", err)
	}
	return &commandFixture{bus: bus, registry: registry, lease: lease}
}

// hub returns a hub on the fixture's state directory whose every request is authenticated as instance "mac"
// with the given transport peer. A different peer models another node claiming the same instance name.
func (f *commandFixture) hub(peer string, poll time.Duration) *Hub {
	return &Hub{
		Bus:       f.bus,
		Instances: f.registry,
		Authenticate: func(r *http.Request) (string, *HubAuthError) {
			setHubAuthenticatedPeer(r, peer)
			return "mac", nil
		},
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

// pollBody encodes a poll request from the start of the journal with the given credentials.
func pollBody(t *testing.T, token string, generation int64) string {
	t.Helper()
	data, err := json.Marshal(commandPollRequest{Version: hubProtocolVersion, Generation: generation, LeaseToken: token, After: 0})
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// ackBody encodes an acknowledgement with the given credentials.
func ackBody(t *testing.T, token string, generation, seq int64, outcome CommandOutcome) string {
	t.Helper()
	data, err := json.Marshal(commandAckRequest{Version: hubProtocolVersion, Generation: generation, LeaseToken: token, Seq: seq, Outcome: outcome})
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// wireCause checks that a reply carries the named cause of op.
func wireCause(t *testing.T, reply commandReply, op, cause string) {
	t.Helper()
	if reply.Error == nil {
		t.Fatalf("expected an error %s/%s, got %+v", op, cause, reply)
	}
	if reply.Error.Op != op || reply.Error.Cause != cause {
		t.Fatalf("error = %s/%s, want %s/%s (%s)", reply.Error.Op, reply.Error.Cause, op, cause, reply.Error.Message)
	}
}

// enqueueStop queues a valid stop_task command for task t1; tests use it when they only need a pending command.
func enqueueStop(t *testing.T, h *Hub) {
	t.Helper()
	if err := h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "test"}); err != nil {
		t.Fatal(err)
	}
}

func TestCommandEnqueueRejectsUnknownKind(t *testing.T) {
	h := newCommandFixture(t).hub("token:mac", 0)
	failsWith(t, CommandEnqueueUnknownKind)("enqueue", h.EnqueueCommand("mac", "reboot", "t1", nil))
}

func TestCommandEnqueueRejectsMissingInstanceOrTask(t *testing.T) {
	h := newCommandFixture(t).hub("token:mac", 0)
	failsWith(t, CommandEnqueueInvalidParams)("enqueue", h.EnqueueCommand("", CommandStopTask, "t1", map[string]string{"reason": "x"}))
	failsWith(t, CommandEnqueueInvalidParams)("enqueue", h.EnqueueCommand("mac", CommandStopTask, "", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueRejectsMissingRequiredParameter(t *testing.T) {
	h := newCommandFixture(t).hub("token:mac", 0)
	failsWith(t, CommandEnqueueInvalidParams)("enqueue", h.EnqueueCommand("mac", CommandStartRun, "t1", map[string]string{"agent": "claude"}))
}

func TestCommandEnqueueFailsWhenInstanceHasNoLease(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	failsWith(t, CommandEnqueueInstanceUnreachable)("enqueue", h.EnqueueCommand("ghost", CommandStopTask, "t1", map[string]string{"reason": "x"}))
	noRegistry := &Hub{Bus: f.bus}
	failsWith(t, CommandEnqueueInstanceUnreachable)("enqueue", noRegistry.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueFailsOnRegistryStorageError(t *testing.T) {
	f := newCommandFixture(t)
	// A directory where the registry's state file belongs cannot be read as a file.
	if err := os.Remove(f.bus.path("state.json")); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(f.bus.path("state.json"), 0o700); err != nil {
		t.Fatal(err)
	}
	failsWith(t, CommandEnqueueStorage)("enqueue", f.hub("token:mac", 0).EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueFailsWhenQueueIsFull(t *testing.T) {
	h := newCommandFixture(t).hub("token:mac", 0)
	for i := 0; i < commandQueueLimit; i++ {
		enqueueStop(t, h)
	}
	failsWith(t, CommandEnqueueQueueFull)("enqueue", h.EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueFailsOnJournalStorageError(t *testing.T) {
	f := newCommandFixture(t)
	// A directory where the journal belongs cannot be read as a file.
	if err := os.Mkdir(f.bus.path(commandStoreFile), 0o700); err != nil {
		t.Fatal(err)
	}
	failsWith(t, CommandEnqueueStorage)("enqueue", f.hub("token:mac", 0).EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}))
}

func TestCommandEnqueueLogsTheQueuedCommand(t *testing.T) {
	logs := captureLogs(t)
	h := newCommandFixture(t).hub("token:mac", 0)
	if err := h.EnqueueCommand("mac", CommandWake, "t1", map[string]string{"member": "codex-1", "reason": "message"}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(logs.String(), `"event":"command_queued"`) {
		t.Fatalf("no command_queued record: %s", logs.String())
	}
}

func TestCommandPollDeliversQueuedCommandsInOrder(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	if err := h.EnqueueCommand("mac", CommandStartRun, "t1", map[string]string{"agent": "claude-1", "run": "1"}); err != nil {
		t.Fatal(err)
	}
	enqueueStop(t, h)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
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
	f := newCommandFixture(t)
	h := f.hub("token:mac", 50*time.Millisecond)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
	if w.Code != http.StatusNoContent || w.Body.Len() != 0 {
		t.Fatalf("empty poll: status %d body %q", w.Code, w.Body.String())
	}
}

func TestCommandPollWakesWhenCommandArrives(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 5*time.Second)
	go func() {
		time.Sleep(100 * time.Millisecond)
		if err := h.EnqueueCommand("mac", CommandStopTask, "t9", map[string]string{"reason": "stop"}); err != nil {
			t.Error("enqueue in test goroutine:", err)
		}
	}()
	start := time.Now()
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
	reply := decodeReply(t, w)
	if w.Code != http.StatusOK || len(reply.Commands) != 1 || reply.Commands[0].Task != "t9" {
		t.Fatalf("long poll: status %d %+v", w.Code, reply)
	}
	if time.Since(start) >= 5*time.Second {
		t.Fatal("the poll waited for the whole window instead of returning the new command")
	}
}

func TestCommandPollRejectsWrongPeer(t *testing.T) {
	f := newCommandFixture(t)
	enqueueStop(t, f.hub("token:mac", 0))
	// Another node that knows the token but authenticates as a different peer must not read the commands.
	intruder := f.hub("token:other", 0)
	w := postCommand(t, intruder, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
	if w.Code != http.StatusForbidden {
		t.Fatalf("wrong peer: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "NotLeaseOwner")
}

// A wrong token is a credential that no longer matches the lease, so the registry reports it as stale.
func TestCommandPollRejectsWrongToken(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	enqueueStop(t, h)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, strings.Repeat("0", 64), f.lease.Generation))
	if w.Code != http.StatusConflict {
		t.Fatalf("wrong token: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "LeaseStale")
}

func TestCommandPollRejectsStaleLease(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation+1))
	reply := decodeReply(t, w)
	if w.Code != http.StatusConflict {
		t.Fatalf("stale generation: status %d", w.Code)
	}
	wireCause(t, reply, "command_poll", "LeaseStale")
	if cause, ok := CommandPollCauseByName(reply.Error.Cause); !ok || cause != CommandPollLeaseStale {
		t.Fatalf("cause name does not rebuild the typed cause: %v %v", cause, ok)
	}
}

func TestCommandPollFailsClosedWithoutRegistry(t *testing.T) {
	f := newCommandFixture(t)
	h := &Hub{Bus: f.bus, Authenticate: f.hub("token:mac", 0).Authenticate}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
	if w.Code != http.StatusConflict {
		t.Fatalf("no registry: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "LeaseStale")
}

func TestCommandPollRejectsMalformedRequests(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	for name, body := range map[string]string{
		"not JSON":      "{",
		"unknown field": `{"version":1,"generation":1,"lease_token":"x","after":0,"extra":true}`,
		"two objects":   pollBody(t, f.lease.Token, f.lease.Generation) + pollBody(t, f.lease.Token, f.lease.Generation),
		"wrong version": `{"version":99,"generation":1,"lease_token":"x","after":0}`,
	} {
		w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", body)
		if w.Code != http.StatusBadRequest {
			t.Fatalf("%s: status %d", name, w.Code)
		}
		wireCause(t, decodeReply(t, w), "command_poll", "Malformed")
	}
}

func TestCommandRoutesRejectOtherMethodsAndPaths(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
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
	f := newCommandFixture(t)
	if err := f.hub("token:mac", 0).EnqueueCommand("mac", CommandWake, "t1", map[string]string{"member": "codex-1", "reason": "board"}); err != nil {
		t.Fatal(err)
	}
	restarted := &Hub{Bus: f.bus, Instances: &HubInstanceRegistry{Bus: f.bus}, Authenticate: f.hub("token:mac", 0).Authenticate}
	reply := decodeReply(t, postCommand(t, restarted, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation)))
	if len(reply.Commands) != 1 || reply.Commands[0].Kind != CommandWake {
		t.Fatalf("command lost across restart: %+v", reply)
	}
}

func TestCommandPollFailsOnJournalStorageError(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 50*time.Millisecond)
	if err := os.Mkdir(f.bus.path(commandStoreFile), 0o700); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("storage failure: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "Storage")
}

func TestCommandPollFailsOnRegistryStorageError(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	if err := os.Remove(f.bus.path("state.json")); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(f.bus.path("state.json"), 0o700); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation))
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("registry storage failure: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_poll", "Storage")
}

func TestCommandAckRemovesCommandAndRecordsOutcome(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 50*time.Millisecond)
	if err := h.EnqueueCommand("mac", CommandStartRun, "t1", map[string]string{"agent": "claude-1", "run": "3"}); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStarted))
	if w.Code != http.StatusOK {
		t.Fatalf("ack: status %d %s", w.Code, w.Body.String())
	}
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation)); w.Code != http.StatusNoContent {
		t.Fatalf("acknowledged command was delivered again: status %d", w.Code)
	}
	store, serr := f.bus.loadCommandStore()
	if serr != nil {
		t.Fatal(serr)
	}
	results := store.Queues["mac"].Results
	if len(results) != 1 || results[0].Seq != 1 || results[0].Outcome != CommandOutcomeStarted || results[0].Kind != CommandStartRun {
		t.Fatalf("result not recorded: %+v", results)
	}
}

func TestCommandAckKeepsOnlyRecentResults(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	for i := 0; i < commandResultLimit+5; i++ {
		enqueueStop(t, h)
		if w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, int64(i+1), CommandOutcomeStopped)); w.Code != http.StatusOK {
			t.Fatalf("ack %d: status %d", i, w.Code)
		}
	}
	store, serr := f.bus.loadCommandStore()
	if serr != nil {
		t.Fatal(serr)
	}
	results := store.Queues["mac"].Results
	if len(results) != commandResultLimit || results[0].Seq != 6 {
		t.Fatalf("results kept: %d, first %d", len(results), results[0].Seq)
	}
}

func TestCommandAckErrors(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	enqueueStop(t, h)
	cases := []struct {
		name   string
		hub    *Hub
		body   string
		status int
		cause  string
	}{
		{"unknown sequence number", h, ackBody(t, f.lease.Token, f.lease.Generation, 99, CommandOutcomeStopped), http.StatusNotFound, "NoSuchCommand"},
		{"unknown outcome", h, ackBody(t, f.lease.Token, f.lease.Generation, 1, "exploded"), http.StatusBadRequest, "InvalidOutcome"},
		{"wrong peer", f.hub("token:other", 0), ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStopped), http.StatusForbidden, "NotLeaseOwner"},
		{"wrong token", h, ackBody(t, strings.Repeat("0", 64), f.lease.Generation, 1, CommandOutcomeStopped), http.StatusConflict, "LeaseStale"},
		{"stale lease", h, ackBody(t, f.lease.Token, f.lease.Generation+1, 1, CommandOutcomeStopped), http.StatusConflict, "LeaseStale"},
		{"malformed body", h, "not json", http.StatusBadRequest, "Malformed"},
	}
	for _, c := range cases {
		w := postCommand(t, c.hub, http.MethodPost, "/v1/commands/ack", c.body)
		if w.Code != c.status {
			t.Fatalf("%s: status %d, want %d", c.name, w.Code, c.status)
		}
		wireCause(t, decodeReply(t, w), "command_ack", c.cause)
	}
	// The command is still pending after every failed acknowledgement.
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation)); w.Code != http.StatusOK {
		t.Fatalf("pending command lost after failed acks: status %d", w.Code)
	}
}

func TestCommandAckRejectsSecondAcknowledgement(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	enqueueStop(t, h)
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStopped)); w.Code != http.StatusOK {
		t.Fatalf("first ack: status %d", w.Code)
	}
	// An identical retry after a lost response is accepted, and it does not record a second result.
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStopped)); w.Code != http.StatusOK {
		t.Fatalf("identical retry: status %d %s", w.Code, w.Body.String())
	}
	// A retry with a different outcome for the same command is a conflict, not a second answer.
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeFailed))
	if w.Code != http.StatusConflict {
		t.Fatalf("conflicting retry: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_ack", "ConflictingOutcome")
	store, serr := f.bus.loadCommandStore()
	if serr != nil {
		t.Fatal(serr)
	}
	if results := store.Queues["mac"].Results; len(results) != 1 || results[0].Outcome != CommandOutcomeStopped {
		t.Fatalf("retries changed the recorded result: %+v", results)
	}
}

// A retry must match the detail too, so a failed outcome with a different reason is a conflict.
func TestCommandAckRetryComparesDetail(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	enqueueStop(t, h)
	if w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStopped)); w.Code != http.StatusOK {
		t.Fatalf("first ack: status %d", w.Code)
	}
	data, err := json.Marshal(commandAckRequest{Version: hubProtocolVersion, Generation: f.lease.Generation, LeaseToken: f.lease.Token, Seq: 1, Outcome: CommandOutcomeStopped, Detail: "other"})
	if err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", string(data))
	wireCause(t, decodeReply(t, w), "command_ack", "ConflictingOutcome")
}

// Commands handed out by a poll carry the lease generation the hub checked for that poll.
func TestCommandPollStampsTheDeliveredGeneration(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	enqueueStop(t, h)
	reply := decodeReply(t, postCommand(t, h, http.MethodPost, "/v1/commands/poll", pollBody(t, f.lease.Token, f.lease.Generation)))
	if len(reply.Commands) != 1 || reply.Commands[0].Generation != f.lease.Generation {
		t.Fatalf("delivered command without its generation: %+v", reply.Commands)
	}
}

func TestCommandAckFailsOnJournalStorageError(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	if err := os.Mkdir(f.bus.path(commandStoreFile), 0o700); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStopped))
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("storage failure: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_ack", "Storage")
}

func TestCommandAckFailsOnRegistryStorageError(t *testing.T) {
	f := newCommandFixture(t)
	h := f.hub("token:mac", 0)
	enqueueStop(t, h)
	if err := os.Remove(f.bus.path("state.json")); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(f.bus.path("state.json"), 0o700); err != nil {
		t.Fatal(err)
	}
	w := postCommand(t, h, http.MethodPost, "/v1/commands/ack", ackBody(t, f.lease.Token, f.lease.Generation, 1, CommandOutcomeStopped))
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("registry storage failure: status %d", w.Code)
	}
	wireCause(t, decodeReply(t, w), "command_ack", "Storage")
}

func TestCommandJournalKeepsItsFormatVersion(t *testing.T) {
	f := newCommandFixture(t)
	if err := f.hub("token:mac", 0).EnqueueCommand("mac", CommandStopTask, "t1", map[string]string{"reason": "x"}); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(f.bus.path(commandStoreFile))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(data, []byte(`"version": 1`)) {
		t.Fatalf("journal without a version: %s", data)
	}
	// A journal written by a newer protocol version is refused, not reinterpreted.
	if err := os.WriteFile(f.bus.path(commandStoreFile), []byte(`{"version":2,"queues":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, serr := f.bus.loadCommandStore(); serr == nil {
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
