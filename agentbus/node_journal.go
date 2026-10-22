// The node command journal prevents redelivery from blindly repeating local process side effects.
package main

import (
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
)

// nodeCommandPhase distinguishes accepted work from durable outcomes and confirmed acknowledgments.
type nodeCommandPhase string

const (
	nodeCommandAccepted     nodeCommandPhase = "accepted"     // side effects may or may not have happened after a crash
	nodeCommandCompleted    nodeCommandPhase = "completed"    // result is durable and must be acknowledged without execution
	nodeCommandAcknowledged nodeCommandPhase = "acknowledged" // hub confirmed receipt
)

// Valid reports whether p is a persisted node journal state.
func (p nodeCommandPhase) Valid() bool {
	return p == nodeCommandAccepted || p == nodeCommandCompleted || p == nodeCommandAcknowledged
}

// UnmarshalJSON implements json.Unmarshaler; errors contain *ValueError.
func (p *nodeCommandPhase) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, p, "node command phase").asError()
}

// NodeExecutionResult is a local outcome, durably recorded before acknowledging the hub.
type NodeExecutionResult struct {
	Outcome CommandOutcome `json:"outcome"` // typed result known by the command channel
	Detail  string         `json:"detail"`  // diagnostic only; never executed
}

// nodeCommandRecord retains accepted payload identity and the monotonic execution outcome.
type nodeCommandRecord struct {
	Digest  string              `json:"digest"`  // detects conflicting sequence reuse
	Phase   nodeCommandPhase    `json:"phase"`   // accepted/completed/acknowledged
	Command Command             `json:"command"` // exact accepted instruction, for crash reconciliation
	Result  NodeExecutionResult `json:"result"`  // present only after completion
}

// nodeState is local-only state, never a client-side copy of the logical bus.
type nodeState struct {
	Version        int                           `json:"version"`         // new node file version
	Instance       string                        `json:"instance_name"`   // changing identity with active journal state is refused
	RegistrationID string                        `json:"registration_id"` // stable across reconnect and daemon restart
	Sequence       uint64                        `json:"sequence"`        // greatest locally issued manifest report sequence
	Lease          HubInstanceLease              `json:"lease"`           // latest hub-issued credentials
	After          int64                         `json:"after"`           // greatest durably confirmed command acknowledgment
	Commands       map[string]*nodeCommandRecord `json:"commands"`        // retained outcomes fence duplicate execution
}

// lockNodeHome excludes a second daemon in this local home without relying on the external flock utility.
func lockNodeHome(home string) (*os.File, *CmdNodeError) {
	dir := filepath.Join(home, "node")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fromStorage(CmdNodeStorage, storageErr(err, "cannot create node directory"))
	}
	if serr := syncHubDirectory(home); serr != nil {
		return nil, fromStorage(CmdNodeStorage, serr)
	}
	file, err := os.OpenFile(filepath.Join(dir, "daemon.lock"), os.O_CREATE|os.O_RDWR, 0o600) //nolint:gosec // G304: fixed lock filename under operator-selected absolute AGENTS_HOME, never a hub-supplied path
	if err != nil {
		return nil, fromStorage(CmdNodeStorage, storageErr(err, "cannot open node lock"))
	}
	if err := syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		closeHubFile(file)
		if errors.Is(err, syscall.EWOULDBLOCK) {
			return nil, opErr(CmdNodeAlreadyRunning, "another node daemon owns this home")
		}
		return nil, fromStorage(CmdNodeStorage, storageErr(err, "cannot lock node home"))
	}
	return file, nil
}

// loadNodeState accepts an absent file but never resets corrupt or unknown journal contents.
func loadNodeState(home, instance string) (*nodeState, *StorageError) {
	data, err := os.ReadFile(filepath.Join(home, "node", "state.json")) //nolint:gosec // G304: fixed journal filename under operator-selected absolute AGENTS_HOME
	if errors.Is(err, os.ErrNotExist) {
		return &nodeState{Version: hubProtocolVersion, Instance: instance, Commands: make(map[string]*nodeCommandRecord)}, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read node journal")
	}
	var state nodeState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, storageErr(err, "cannot decode node journal")
	}
	if state.Version != hubProtocolVersion || state.Instance != instance || state.Commands == nil || state.After < 0 {
		return nil, storageErr(valueErr("invalid node version, identity, or cursor"), "cannot validate node journal")
	}
	for seq, record := range state.Commands {
		if record == nil || seq != fmt.Sprint(record.Command.Seq) || record.Command.Seq <= 0 || record.Digest == "" || !record.Phase.Valid() || (record.Phase != nodeCommandAccepted && !record.Result.Outcome.Valid()) {
			return nil, storageErr(valueErr("invalid node command record %q", seq), "cannot validate node journal")
		}
	}
	return &state, nil
}

// saveNodeState syncs the journal and its directory before process execution or acknowledgment advances.
func saveNodeState(home string, state *nodeState) *StorageError {
	data, err := json.Marshal(state)
	if err != nil {
		return storageErr(err, "cannot encode node journal")
	}
	dir := filepath.Join(home, "node")
	file, err := os.CreateTemp(dir, "state-*.tmp")
	if err != nil {
		return storageErr(err, "cannot create node journal temporary file")
	}
	defer func() {
		if err := os.Remove(file.Name()); err != nil && !errors.Is(err, os.ErrNotExist) {
			warnSideEffect("remove node journal temporary file", storageErr(err, "cannot remove temporary journal"))
		}
	}()
	if _, err := file.Write(data); err != nil {
		closeHubFile(file)
		return storageErr(err, "cannot write node journal")
	}
	if err := file.Sync(); err != nil {
		closeHubFile(file)
		return storageErr(err, "cannot sync node journal")
	}
	if err := file.Close(); err != nil {
		return storageErr(err, "cannot close node journal")
	}
	if err := os.Rename(file.Name(), filepath.Join(dir, "state.json")); err != nil {
		return storageErr(err, "cannot replace node journal")
	}
	return syncHubDirectory(dir)
}

// nodeCommandDigest excludes a refreshed delivery generation but includes all executable parameters.
func nodeCommandDigest(command Command) (string, *ValueError) {
	command.Generation = 0
	data, err := json.Marshal(command)
	if err != nil {
		return "", valueWrap(err, "cannot encode node command")
	}
	return fmt.Sprintf("%x", sha256.Sum256(data)), nil
}
