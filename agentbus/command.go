// Package main provides the command channel between the hub and agent node daemons.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

// NodeCommandKind distinguishes the action requested of an agent node.
type NodeCommandKind string

const (
	// CmdStartRun instructs the node to start a runner process for a task.
	CmdStartRun NodeCommandKind = "start_run"
	// CmdWake instructs the node to wake a sleeping agent for a message or board item.
	CmdWake NodeCommandKind = "wake"
	// CmdStopTask instructs the node to terminate a task's execution.
	CmdStopTask NodeCommandKind = "stop_task"
	// CmdAnswerDeliver delivers human answers to a waiting coordinator task.
	CmdAnswerDeliver NodeCommandKind = "answer_deliver"
)

// Valid reports whether k is a recognized node command kind.
func (k NodeCommandKind) Valid() bool {
	switch k {
	case CmdStartRun, CmdWake, CmdStopTask, CmdAnswerDeliver:
		return true
	}
	return false
}

// NodeCommandStatus reports the delivery and acknowledgment lifecycle of a command.
type NodeCommandStatus string

const (
	// CmdPending means the command is queued waiting for node poll and acknowledgment.
	CmdPending NodeCommandStatus = "pending"
	// CmdAcked means the node executed and acknowledged the command.
	CmdAcked NodeCommandStatus = "acked"
)

// Valid reports whether s is a recognized command status.
func (s NodeCommandStatus) Valid() bool {
	return s == CmdPending || s == CmdAcked
}

// NodeCommand represents an instruction queued by the hub for a remote agent node.
type NodeCommand struct {
	ID        string            `json:"id"`                   // unique command identifier (e.g. "cmd-1")
	Instance  string            `json:"instance"`             // destination instance name
	Lease     uint64            `json:"lease,omitempty"`      // lease generation when command was queued
	Kind      NodeCommandKind   `json:"kind"`                 // action type
	Task      string            `json:"task,omitempty"`       // task ID for start_run, stop_task, or answer_deliver
	Agent     string            `json:"agent,omitempty"`      // target agent name for start_run or wake
	Run       int               `json:"run,omitempty"`        // run number for start_run
	PromptRef string            `json:"prompt_ref,omitempty"` // prompt template or file reference
	Reason    string            `json:"reason,omitempty"`     // reason for wake or stop_task
	Answer    string            `json:"answer,omitempty"`     // human answer content for answer_deliver
	Status    NodeCommandStatus `json:"status"`               // pending or acked
	AckResult string            `json:"ack_result,omitempty"` // result reported by node (started, stopped, failed:...)
	CreatedAt Timestamp         `json:"created_at"`           // timestamp when command was enqueued
	AckedAt   Timestamp         `json:"acked_at,omitempty"`   // timestamp when command was acknowledged
}

// CommandStore persists queued and acknowledged node commands in bus/commands.json.
type CommandStore struct {
	Commands map[string]*NodeCommand `json:"commands"` // all commands keyed by command ID
	NextID   uint64                  `json:"next_id"`  // monotonic command counter
}

// CommandPollRequest is the payload for POST /v1/commands/poll.
type CommandPollRequest struct {
	Version      int    `json:"version"`       // protocol version (1)
	InstanceName string `json:"instance_name"` // authenticated instance name
	Lease        uint64 `json:"lease"`         // current active lease generation
	LastCursor   uint64 `json:"last_cursor"`   // last acknowledged command cursor
}

// CommandPollResponse is the payload returned by POST /v1/commands/poll when commands are available.
type CommandPollResponse struct {
	Version  int           `json:"version"`  // protocol version (1)
	Commands []NodeCommand `json:"commands"` // batch of pending commands
}

// CommandAckRequest is the payload for POST /v1/commands/ack.
type CommandAckRequest struct {
	Version      int    `json:"version"`       // protocol version (1)
	InstanceName string `json:"instance_name"` // authenticated instance name
	Lease        uint64 `json:"lease"`         // current active lease generation
	CommandID    string `json:"command_id"`    // command ID to acknowledge
	Result       string `json:"result"`        // execution result (started, already_running, stopped, unknown_task, failed:<reason>)
}

// CommandAckResponse is the result of POST /v1/commands/ack.
type CommandAckResponse struct {
	Version   int    `json:"version"`    // protocol version (1)
	CommandID string `json:"command_id"` // acknowledged command ID
	Status    string `json:"status"`     // acked
}

// ValidCommandAckResult reports whether res matches allowed command execution result formats.
func ValidCommandAckResult(res string) bool {
	switch res {
	case "started", "already_running", "stopped", "unknown_task":
		return true
	}
	return strings.HasPrefix(res, "failed:")
}

// CommandManager coordinates command queueing, long polling, and durable acknowledgments.
type CommandManager struct {
	bus       *Bus                       // underlying bus directory
	instances *InstanceManager           // instance lease and manifest tracker
	waiters   map[string][]chan struct{} // waiting long-poll listeners per instance
	mu        sync.Mutex                 // guards in-memory state and waiters
}

// NewCommandManager creates a command manager for the given bus and instance manager.
func NewCommandManager(b *Bus, im *InstanceManager) *CommandManager {
	return &CommandManager{
		bus:       b,
		instances: im,
		waiters:   make(map[string][]chan struct{}),
	}
}

// load reads bus/commands.json or returns an empty store if it does not exist.
func (cm *CommandManager) load() (*CommandStore, *StorageError) {
	p := cm.bus.path("commands.json")
	data, err := os.ReadFile(p) //nolint:gosec // p is inside the managed bus state directory
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return &CommandStore{
				Commands: make(map[string]*NodeCommand),
				NextID:   1,
			}, nil
		}
		return nil, storageErr(err, "cannot read %s", p)
	}
	var store CommandStore
	if err := json.Unmarshal(data, &store); err != nil {
		return nil, storageErr(err, "cannot decode %s", p)
	}
	if store.Commands == nil {
		store.Commands = make(map[string]*NodeCommand)
	}
	if store.NextID == 0 {
		store.NextID = 1
	}
	return &store, nil
}

// save atomically writes bus/commands.json using a temporary file.
func (cm *CommandManager) save(store *CommandStore) *StorageError {
	p := cm.bus.path("commands.json")
	data, err := json.MarshalIndent(store, "", "  ")
	if err != nil {
		return storageErr(err, "cannot marshal commands")
	}
	tmp := fmt.Sprintf("%s.tmp.%d", p, time.Now().UnixNano())
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return storageErr(err, "cannot write %s", tmp)
	}
	if err := os.Rename(tmp, p); err != nil {
		if rerr := os.Remove(tmp); rerr != nil {
			warnSideEffect("clean temporary commands file", storageErr(rerr, "cannot remove %s", tmp))
		}
		return storageErr(err, "cannot rename %s to %s", tmp, p)
	}
	return nil
}

// notifyWaiters wakes all waiting long polls for the given instance.
func (cm *CommandManager) notifyWaiters(instance string) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	channels := cm.waiters[instance]
	for _, ch := range channels {
		close(ch)
	}
	delete(cm.waiters, instance)
}

// registerWaiter creates a channel that will be closed when a command is queued for instance.
func (cm *CommandManager) registerWaiter(instance string) chan struct{} {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	ch := make(chan struct{})
	cm.waiters[instance] = append(cm.waiters[instance], ch)
	return ch
}

// removeWaiter unregisters a waiting listener if it timed out.
func (cm *CommandManager) removeWaiter(instance string, ch chan struct{}) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	list := cm.waiters[instance]
	for i, c := range list {
		if c == ch {
			cm.waiters[instance] = append(list[:i], list[i+1:]...)
			break
		}
	}
	if len(cm.waiters[instance]) == 0 {
		delete(cm.waiters, instance)
	}
}

// Enqueue adds a command to the queue, persists it, and notifies waiting polls.
func (cm *CommandManager) Enqueue(cmd NodeCommand) (*NodeCommand, *CommandEnqueueError) {
	if !cmd.Kind.Valid() {
		return nil, opErr(CommandEnqueueInvalidCommand, "unknown command kind %q", cmd.Kind)
	}
	if cmd.Instance == "" {
		return nil, opErr(CommandEnqueueInvalidCommand, "instance name is required")
	}

	// Verify that the destination instance is registered and active
	instRec, ok := cm.instances.GetInstance(cmd.Instance)
	if !ok || instRec.Status != InstanceActive || !instRec.LeaseExpiresAt.After(time.Now()) {
		return nil, opErr(CommandEnqueueInstanceUnreachable, "instance %q is unreachable or has an expired lease", cmd.Instance)
	}

	cmd.Lease = instRec.Lease

	cm.mu.Lock()
	store, serr := cm.load()
	if serr != nil {
		cm.mu.Unlock()
		return nil, fromStorage(CommandEnqueueStorage, serr)
	}

	if cmd.ID == "" {
		cmd.ID = "cmd-" + strconv.FormatUint(store.NextID, 10)
		store.NextID++
	} else if _, exists := store.Commands[cmd.ID]; exists {
		cm.mu.Unlock()
		return nil, opErr(CommandEnqueueDuplicateCommand, "command %q already exists", cmd.ID)
	}

	cmd.Status = CmdPending
	cmd.CreatedAt = Timestamp{time.Now()}
	store.Commands[cmd.ID] = &cmd

	if serr := cm.save(store); serr != nil {
		cm.mu.Unlock()
		return nil, fromStorage(CommandEnqueueStorage, serr)
	}
	cm.mu.Unlock()

	cm.notifyWaiters(cmd.Instance)
	return &cmd, nil
}

// Poll checks for pending commands for the instance or waits up to timeout for one to arrive.
func (cm *CommandManager) Poll(ctx context.Context, authenticatedInstance string, req CommandPollRequest, timeout time.Duration) (*CommandPollResponse, *CommandPollError) {
	if req.Version != hubProtocolVersion {
		return nil, opErr(CommandPollMalformed, "unsupported protocol version %d", req.Version)
	}
	if req.InstanceName == "" {
		return nil, opErr(CommandPollMalformed, "instance_name cannot be empty")
	}
	if authenticatedInstance != "" && req.InstanceName != authenticatedInstance {
		return nil, opErr(CommandPollInstanceMismatch, "poll instance_name %q does not match authenticated instance %q", req.InstanceName, authenticatedInstance)
	}

	instRec, ok := cm.instances.GetInstance(req.InstanceName)
	if !ok || instRec.Status != InstanceActive || !instRec.LeaseExpiresAt.After(time.Now()) {
		return nil, opErr(CommandPollInstanceUnreachable, "instance %q is not registered or lease has expired", req.InstanceName)
	}
	if req.Lease != instRec.Lease {
		return nil, opErr(CommandPollStaleLease, "polling lease %d does not match active lease %d", req.Lease, instRec.Lease)
	}

	getPending := func() ([]NodeCommand, *CommandPollError) {
		cm.mu.Lock()
		defer cm.mu.Unlock()

		store, serr := cm.load()
		if serr != nil {
			return nil, fromStorage(CommandPollStorage, serr)
		}

		var pending []NodeCommand
		for _, cmd := range store.Commands {
			if cmd.Instance == req.InstanceName && cmd.Status == CmdPending {
				pending = append(pending, *cmd)
			}
		}
		return pending, nil
	}

	pending, err := getPending()
	if err != nil {
		return nil, err
	}
	if len(pending) > 0 {
		return &CommandPollResponse{
			Version:  hubProtocolVersion,
			Commands: pending,
		}, nil
	}

	if timeout <= 0 {
		return &CommandPollResponse{
			Version:  hubProtocolVersion,
			Commands: nil,
		}, nil
	}

	// Wait for a new command or timeout
	waiter := cm.registerWaiter(req.InstanceName)
	defer cm.removeWaiter(req.InstanceName, waiter)

	select {
	case <-ctx.Done():
		return &CommandPollResponse{
			Version:  hubProtocolVersion,
			Commands: nil,
		}, nil
	case <-time.After(timeout):
		return &CommandPollResponse{
			Version:  hubProtocolVersion,
			Commands: nil,
		}, nil
	case <-waiter:
		// Woken by notifyWaiters: re-check for pending commands
		pendingAfter, errAfter := getPending()
		if errAfter != nil {
			return nil, errAfter
		}
		return &CommandPollResponse{
			Version:  hubProtocolVersion,
			Commands: pendingAfter,
		}, nil
	}
}

// Ack records the execution outcome of a node command.
func (cm *CommandManager) Ack(authenticatedInstance string, req CommandAckRequest) (*CommandAckResponse, *CommandAckError) {
	if req.Version != hubProtocolVersion {
		return nil, opErr(CommandAckMalformed, "unsupported protocol version %d", req.Version)
	}
	if req.InstanceName == "" || req.CommandID == "" {
		return nil, opErr(CommandAckMalformed, "instance_name and command_id are required")
	}
	if authenticatedInstance != "" && req.InstanceName != authenticatedInstance {
		return nil, opErr(CommandAckInstanceMismatch, "ack instance_name %q does not match authenticated instance %q", req.InstanceName, authenticatedInstance)
	}
	if !ValidCommandAckResult(req.Result) {
		return nil, opErr(CommandAckInvalidResult, "invalid command execution result %q", req.Result)
	}

	instRec, ok := cm.instances.GetInstance(req.InstanceName)
	if !ok || instRec.Status != InstanceActive || !instRec.LeaseExpiresAt.After(time.Now()) {
		return nil, opErr(CommandAckInstanceUnreachable, "instance %q is not registered or lease has expired", req.InstanceName)
	}
	if req.Lease != instRec.Lease {
		return nil, opErr(CommandAckStaleLease, "ack lease %d does not match active lease %d", req.Lease, instRec.Lease)
	}

	cm.mu.Lock()
	defer cm.mu.Unlock()

	store, serr := cm.load()
	if serr != nil {
		return nil, fromStorage(CommandAckStorage, serr)
	}

	cmd, ok := store.Commands[req.CommandID]
	if !ok {
		return nil, opErr(CommandAckCommandNotFound, "command %q not found", req.CommandID)
	}

	if cmd.Status == CmdAcked {
		if cmd.AckResult == req.Result {
			return &CommandAckResponse{
				Version:   hubProtocolVersion,
				CommandID: cmd.ID,
				Status:    string(CmdAcked),
			}, nil
		}
		return nil, opErr(CommandAckConflictAck, "command %q already acknowledged with different result %q", req.CommandID, cmd.AckResult)
	}

	cmd.Status = CmdAcked
	cmd.AckResult = req.Result
	cmd.AckedAt = Timestamp{time.Now()}

	if serr := cm.save(store); serr != nil {
		return nil, fromStorage(CommandAckStorage, serr)
	}

	return &CommandAckResponse{
		Version:   hubProtocolVersion,
		CommandID: cmd.ID,
		Status:    string(CmdAcked),
	}, nil
}
