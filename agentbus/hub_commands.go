// Command channel of the hub: typed commands are queued per instance in bus/commands.json and handed to the
// instance's node daemon in long polls; the node acknowledges each command with its outcome. Delivery is at least
// once: a command stays pending until it is acknowledged, so handlers on the node must be idempotent.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"maps"
	"net/http"
	"os"
	"strings"
	"time"
)

const (
	// commandQueueLimit bounds undelivered commands per instance; enqueueing beyond it fails with QueueFull.
	commandQueueLimit = 256
	// commandResultLimit bounds the acknowledgements kept per instance for diagnosis.
	commandResultLimit = 100
	// commandBatchLimit bounds the commands handed out in one poll response.
	commandBatchLimit = 32
	// commandPollInterval is how often a long poll rechecks the journal; it is never held under the bus lock.
	commandPollInterval = 200 * time.Millisecond
	// commandStoreFile is the hub-only command journal in the bus directory.
	commandStoreFile = "commands.json"
)

// CommandKind is the closed vocabulary of commands a hub sends to a node.
type CommandKind string

const (
	CommandStartRun      CommandKind = "start_run"      // start, or reconcile with an existing, run of one agent in a task
	CommandWake          CommandKind = "wake"           // wake a member on the node (message or board item)
	CommandStopTask      CommandKind = "stop_task"      // stop the runs of a task
	CommandAnswerDeliver CommandKind = "answer_deliver" // deliver a human answer to the coordinator's task
)

// Valid reports whether k is a known command kind.
func (k CommandKind) Valid() bool {
	switch k {
	case CommandStartRun, CommandWake, CommandStopTask, CommandAnswerDeliver:
		return true
	}
	return false
}

// requiredParams lists the parameters a command of this kind must carry; the hub sends parameters, never text.
func (k CommandKind) requiredParams() []string {
	switch k {
	case CommandStartRun:
		return []string{"agent", "run"}
	case CommandWake:
		return []string{"member", "reason"}
	case CommandStopTask:
		return []string{"reason"}
	case CommandAnswerDeliver:
		return []string{"answer"}
	}
	return nil
}

// UnmarshalJSON implements json.Unmarshaler; failures contain *ValueError.
func (k *CommandKind) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, k, "command kind").asError()
}

// CommandOutcome is the closed vocabulary of a node's acknowledgement. Detail carries the reason for OutcomeFailed.
type CommandOutcome string

const (
	CommandOutcomeStarted        CommandOutcome = "started"         // the command launched a run
	CommandOutcomeAlreadyRunning CommandOutcome = "already_running" // the run existed; nothing was launched again
	CommandOutcomeStopped        CommandOutcome = "stopped"         // the task's runs were stopped
	CommandOutcomeUnknownTask    CommandOutcome = "unknown_task"    // the node has no such task
	CommandOutcomeDelivered      CommandOutcome = "delivered"       // wake and answer_deliver: handed to the member
	CommandOutcomeFailed         CommandOutcome = "failed"          // the node could not execute it; Detail says why
)

// Valid reports whether o is a known outcome.
func (o CommandOutcome) Valid() bool {
	switch o {
	case CommandOutcomeStarted, CommandOutcomeAlreadyRunning, CommandOutcomeStopped, CommandOutcomeUnknownTask,
		CommandOutcomeDelivered, CommandOutcomeFailed:
		return true
	}
	return false
}

// Command is one instruction queued for an instance. Seq is the instance-local identity: it grows monotonically,
// is never reused, and is the ID the node acknowledges.
type Command struct {
	Seq        int64             `json:"seq"`         // instance-local sequence number
	Kind       CommandKind       `json:"kind"`        // closed vocabulary, see CommandKind
	Task       string            `json:"task"`        // task the command concerns
	Params     map[string]string `json:"params"`      // validated parameters for Kind
	EnqueuedAt time.Time         `json:"enqueued_at"` // UTC; time.Time keeps the instant across time zones
}

// CommandResult is the recorded acknowledgement of one command.
type CommandResult struct {
	Seq     int64          `json:"seq"`      // the acknowledged command
	Kind    CommandKind    `json:"kind"`     // copied from the command for diagnosis
	Task    string         `json:"task"`     // copied from the command for diagnosis
	Outcome CommandOutcome `json:"outcome"`  // the node's answer
	Detail  string         `json:"detail"`   // reason for failed outcomes; empty otherwise
	AckedAt time.Time      `json:"acked_at"` // UTC time the hub recorded the acknowledgement
}

// commandStore is the content of bus/commands.json.
type commandStore struct {
	Version int                      `json:"version"` // file format version, equal to hubProtocolVersion
	Queues  map[string]*commandQueue `json:"queues"`  // by instance name
}

// commandQueue is the command channel state of one instance.
type commandQueue struct {
	Next    int64           `json:"next"`    // last sequence number issued; the next command gets Next+1
	Pending []Command       `json:"pending"` // undelivered or unacknowledged commands, ascending by Seq
	Results []CommandResult `json:"results"` // most recent acknowledgements, at most commandResultLimit
}

// CommandLeases is the part of the instance registry the command channel needs. Live reports whether an
// instance holds an unexpired lease; Current reports whether generation is that lease. The hub fails closed
// (no instance is live, no lease is current) when Leases is nil.
type CommandLeases interface {
	// Live reports whether the instance holds an unexpired lease.
	Live(instance string) bool
	// Current reports whether generation is the generation of the instance's current lease.
	Current(instance string, generation int64) bool
}

// commandPollRequest is the body of POST /v1/commands/poll.
type commandPollRequest struct {
	Version    int   `json:"version"`    // protocol version, equal to hubProtocolVersion
	Generation int64 `json:"generation"` // lease generation the node holds
	After      int64 `json:"after"`      // highest sequence number the node has acknowledged; only later commands are returned
}

// commandAckRequest is the body of POST /v1/commands/ack.
type commandAckRequest struct {
	Version    int            `json:"version"`    // protocol version, equal to hubProtocolVersion
	Generation int64          `json:"generation"` // lease generation the node holds
	Seq        int64          `json:"seq"`        // the command being acknowledged
	Outcome    CommandOutcome `json:"outcome"`    // the node's answer
	Detail     string         `json:"detail"`     // reason for failed outcomes
}

// commandReply is the body of every command-channel response; exactly one of Commands and Error is used,
// and an acknowledgement has neither.
type commandReply struct {
	Version  int           `json:"version"`            // protocol version
	Commands []Command     `json:"commands,omitempty"` // poll batch, in ascending sequence order
	Error    *HubWireError `json:"error,omitempty"`    // typed failure, same wire form as tool calls
}

// queue returns the instance's queue, creating it on first use.
func (s *commandStore) queue(instance string) *commandQueue {
	q := s.Queues[instance]
	if q == nil {
		q = &commandQueue{}
		s.Queues[instance] = q
	}
	return q
}

// loadCommandStore reads bus/commands.json; a missing file is an empty journal, any other failure is a storage error.
func (b *Bus) loadCommandStore() (*commandStore, *StorageError) {
	store := &commandStore{Version: hubProtocolVersion, Queues: map[string]*commandQueue{}}
	data, err := os.ReadFile(b.path(commandStoreFile))
	if errors.Is(err, os.ErrNotExist) {
		return store, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read %s", b.path(commandStoreFile))
	}
	if err := json.Unmarshal(data, store); err != nil {
		return nil, storageErr(err, "cannot decode %s", b.path(commandStoreFile))
	}
	if store.Version != hubProtocolVersion {
		return nil, storageErr(valueErr("unsupported version %d", store.Version), "cannot validate %s", b.path(commandStoreFile))
	}
	if store.Queues == nil {
		store.Queues = map[string]*commandQueue{}
	}
	return store, nil
}

// saveCommandStore replaces bus/commands.json atomically, so a concurrent reader sees the old or the new file.
func (b *Bus) saveCommandStore(store *commandStore) *StorageError {
	data, err := json.MarshalIndent(store, "", "  ")
	if err != nil {
		return storageErr(err, "cannot encode the command journal")
	}
	tmp := b.path(commandStoreFile + ".tmp")
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return storageErr(err, "cannot write %s", tmp)
	}
	if err := os.Rename(tmp, b.path(commandStoreFile)); err != nil {
		return storageErr(err, "cannot replace %s", b.path(commandStoreFile))
	}
	return nil
}

// updateCommands runs fn on the journal under the bus lock and saves it only when fn succeeds. Lock, load and save
// failures become the operation's storage cause.
func updateCommands[C opCause](b *Bus, storage C, fn func(*commandStore) *OpError[C]) *OpError[C] {
	var failed *OpError[C]
	serr := b.locked(func(*State) {
		store, err := b.loadCommandStore()
		if err != nil {
			failed = fromStorage(storage, err)
			return
		}
		if failed = fn(store); failed != nil {
			return
		}
		if err := b.saveCommandStore(store); err != nil {
			failed = fromStorage(storage, err)
		}
	})
	if serr != nil {
		return fromStorage(storage, serr)
	}
	return failed
}

// leaseCurrent reports whether generation is the current lease of instance; without a registry nothing is current.
func (h *Hub) leaseCurrent(instance string, generation int64) bool {
	return h.Leases != nil && h.Leases.Current(instance, generation)
}

// EnqueueCommand queues one command for an instance connected to this hub. Parameters are checked against the kind.
// It fails with InstanceUnreachable when the instance has no live lease, and the command is then not kept.
func (h *Hub) EnqueueCommand(instance string, kind CommandKind, task string, params map[string]string) *CommandEnqueueError {
	if !kind.Valid() {
		return opErr(CommandEnqueueUnknownKind, "unknown command kind %q", string(kind))
	}
	if instance == "" || task == "" {
		return opErr(CommandEnqueueInvalidParams, "a command needs an instance and a task")
	}
	for _, name := range kind.requiredParams() {
		if params[name] == "" {
			return opErr(CommandEnqueueInvalidParams, "command %s needs the parameter %q", kind, name)
		}
	}
	if h.Leases == nil || !h.Leases.Live(instance) {
		return opErr(CommandEnqueueInstanceUnreachable, "instance %q is not connected to the hub; the command was not queued", instance)
	}
	if h.Bus == nil {
		return opErr(CommandEnqueueStorage, "hub bus is not configured")
	}
	var seq int64
	failed := updateCommands(h.Bus, CommandEnqueueStorage, func(store *commandStore) *CommandEnqueueError {
		q := store.queue(instance)
		if len(q.Pending) >= commandQueueLimit {
			return opErr(CommandEnqueueQueueFull, "instance %q has %d undelivered commands; retry later", instance, len(q.Pending))
		}
		q.Next++
		seq = q.Next
		q.Pending = append(q.Pending, Command{Seq: seq, Kind: kind, Task: task, Params: maps.Clone(params), EnqueuedAt: time.Now().UTC()})
		return nil
	})
	if failed == nil {
		logInfo(LogCommandQueued, "instance", instance, "seq", seq, "kind", string(kind), "task", task)
	}
	return failed
}

// pendingCommands returns up to commandBatchLimit unacknowledged commands with a sequence number above after.
// It reads the journal without the bus lock: writers replace the file atomically.
func (b *Bus) pendingCommands(instance string, after int64) ([]Command, *StorageError) {
	store, serr := b.loadCommandStore()
	if serr != nil {
		return nil, serr
	}
	q := store.Queues[instance]
	if q == nil {
		return nil, nil
	}
	var out []Command
	for _, c := range q.Pending {
		if c.Seq > after {
			out = append(out, c)
			if len(out) == commandBatchLimit {
				break
			}
		}
	}
	return out, nil
}

// pollCommands long-polls for commands of an instance. It returns an empty batch when the poll window ends, and
// it rechecks the lease on every round, so a replaced lease ends the poll with LeaseStale.
func (h *Hub) pollCommands(ctx context.Context, instance string, generation, after int64) ([]Command, *CommandPollError) {
	if h.Bus == nil {
		return nil, opErr(CommandPollStorage, "hub bus is not configured")
	}
	poll := h.PollDuration
	if poll <= 0 {
		poll = hubPollDuration
	}
	window := time.NewTimer(poll)
	defer window.Stop()
	ticker := time.NewTicker(commandPollInterval)
	defer ticker.Stop()
	for {
		if !h.leaseCurrent(instance, generation) {
			return nil, opErr(CommandPollLeaseStale, "lease generation %d is not current for instance %q; register again", generation, instance)
		}
		cmds, serr := h.Bus.pendingCommands(instance, after)
		if serr != nil {
			return nil, fromStorage(CommandPollStorage, serr)
		}
		if len(cmds) > 0 {
			return cmds, nil
		}
		select {
		case <-ctx.Done():
			// The node went away; the commands stay queued for its next poll.
			return nil, nil
		case <-window.C:
			return nil, nil
		case <-ticker.C:
		}
	}
}

// ackCommand records the node's outcome for one pending command and removes it from the queue.
func (h *Hub) ackCommand(instance string, generation int64, req commandAckRequest) *CommandAckError {
	if h.Bus == nil {
		return opErr(CommandAckStorage, "hub bus is not configured")
	}
	if !req.Outcome.Valid() {
		return opErr(CommandAckInvalidOutcome, "unknown outcome %q", string(req.Outcome))
	}
	if !h.leaseCurrent(instance, generation) {
		return opErr(CommandAckLeaseStale, "lease generation %d is not current for instance %q; register again", generation, instance)
	}
	var kind CommandKind
	var task string
	failed := updateCommands(h.Bus, CommandAckStorage, func(store *commandStore) *CommandAckError {
		q := store.queue(instance)
		idx := -1
		for i, c := range q.Pending {
			if c.Seq == req.Seq {
				idx = i
				break
			}
		}
		if idx < 0 {
			return opErr(CommandAckNoSuchCommand, "no undelivered command %d for instance %q", req.Seq, instance)
		}
		c := q.Pending[idx]
		kind, task = c.Kind, c.Task
		q.Pending = append(q.Pending[:idx], q.Pending[idx+1:]...)
		q.Results = append(q.Results, CommandResult{Seq: c.Seq, Kind: c.Kind, Task: c.Task, Outcome: req.Outcome,
			Detail: req.Detail, AckedAt: time.Now().UTC()})
		if extra := len(q.Results) - commandResultLimit; extra > 0 {
			q.Results = q.Results[extra:]
		}
		return nil
	})
	if failed == nil {
		logInfo(LogCommandAcked, "instance", instance, "seq", req.Seq, "kind", string(kind), "task", task, "outcome", string(req.Outcome))
	}
	return failed
}

// serveCommands answers the node's command routes. It runs after authentication, so instance is the peer's instance.
func (h *Hub) serveCommands(w http.ResponseWriter, r *http.Request, instance string) {
	if r.Method != http.MethodPost {
		h.writeCommandReply(w, http.StatusBadRequest, commandReply{Version: hubProtocolVersion,
			Error: hubFailure(opErr(HubCallMalformed, "use POST for %s", r.URL.Path))})
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, hubBodyLimit)
	switch r.URL.Path {
	case "/v1/commands/poll":
		h.servePoll(w, r, instance)
	case "/v1/commands/ack":
		h.serveAck(w, r, instance)
	default:
		h.writeCommandReply(w, http.StatusNotFound, commandReply{Version: hubProtocolVersion,
			Error: hubFailure(opErr(HubCallUnknownOperation, "unknown command route %s", r.URL.Path))})
	}
}

// servePoll answers POST /v1/commands/poll: 200 with a batch, 204 when the poll window ended empty, or a typed error.
func (h *Hub) servePoll(w http.ResponseWriter, r *http.Request, instance string) {
	var req commandPollRequest
	if err := decodeCommandBody(r, &req); err != nil {
		writeCommandError(h, w, opErr(CommandPollMalformed, "invalid poll request: %v", err))
		return
	}
	if req.Version != hubProtocolVersion {
		writeCommandError(h, w, opErr(CommandPollMalformed, "unsupported protocol version %d", req.Version))
		return
	}
	cmds, failed := h.pollCommands(r.Context(), instance, req.Generation, req.After)
	if failed != nil {
		writeCommandError(h, w, failed)
		return
	}
	if len(cmds) == 0 {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	h.writeCommandReply(w, http.StatusOK, commandReply{Version: hubProtocolVersion, Commands: cmds})
}

// serveAck answers POST /v1/commands/ack.
func (h *Hub) serveAck(w http.ResponseWriter, r *http.Request, instance string) {
	var req commandAckRequest
	if err := decodeCommandBody(r, &req); err != nil {
		writeCommandError(h, w, opErr(CommandAckMalformed, "invalid acknowledgement: %v", err))
		return
	}
	if req.Version != hubProtocolVersion {
		writeCommandError(h, w, opErr(CommandAckMalformed, "unsupported protocol version %d", req.Version))
		return
	}
	if failed := h.ackCommand(instance, req.Generation, req); failed != nil {
		writeCommandError(h, w, failed)
		return
	}
	h.writeCommandReply(w, http.StatusOK, commandReply{Version: hubProtocolVersion})
}

// writeCommandError writes an operation's typed failure with the HTTP status its class implies.
func writeCommandError[C opCause](h *Hub, w http.ResponseWriter, err *OpError[C]) {
	h.writeCommandReply(w, commandStatus(err.Class()), commandReply{Version: hubProtocolVersion, Error: hubFailure(err)})
}

// writeCommandReply writes one command-channel response.
func (h *Hub) writeCommandReply(w http.ResponseWriter, status int, reply commandReply) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(reply); err != nil {
		warnSideEffect("deliver command reply", storageErr(err, "cannot encode or write command reply"))
	}
}

// commandStatus maps an error class to the HTTP status of a command-channel response.
func commandStatus(c ErrorClass) int {
	switch c {
	case ClassInvalidArgument:
		return http.StatusBadRequest
	case ClassPermission:
		return http.StatusForbidden
	case ClassNotFound:
		return http.StatusNotFound
	case ClassInvalidState, ClassConflict:
		return http.StatusConflict
	default:
		return http.StatusServiceUnavailable
	}
}

// decodeCommandBody reads exactly one JSON object with no unknown fields; the returned error is a *ValueError.
func decodeCommandBody(r *http.Request, v any) *ValueError {
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		return valueWrap(err, "the body must be one JSON object")
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return valueErr("the body must contain exactly one JSON object")
	}
	return nil
}

// isCommandRoute reports whether a request path belongs to the command channel.
func isCommandRoute(path string) bool {
	return strings.HasPrefix(path, "/v1/commands/")
}
