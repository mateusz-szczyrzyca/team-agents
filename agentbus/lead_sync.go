package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
	"unicode/utf8"
)

// SyncMetadataKey is one permitted display-only task metadata key.
type SyncMetadataKey string

const (
	SyncStatus      SyncMetadataKey = "STATUS"       // last known local task state, including interrupted
	SyncRun         SyncMetadataKey = "RUN"          // current run number
	SyncAgent       SyncMetadataKey = "AGENT"        // configured agent name
	SyncType        SyncMetadataKey = "TYPE"         // CLI type
	SyncModel       SyncMetadataKey = "MODEL"        // model display name
	SyncNote        SyncMetadataKey = "NOTE"         // human-visible task note
	SyncPauseReason SyncMetadataKey = "PAUSE_REASON" // reason a task paused
	SyncWakeAt      SyncMetadataKey = "WAKE_AT"      // scheduled wake epoch
	SyncCreated     SyncMetadataKey = "CREATED"      // creation timestamp
	SyncFinished    SyncMetadataKey = "FINISHED"     // completion timestamp
	SyncRunStart    SyncMetadataKey = "RUN_START"    // runner start timestamp
	SyncJobID       SyncMetadataKey = "JOB_ID"       // owning job identifier
	SyncRestarts    SyncMetadataKey = "RESTARTS"     // number of machine-restart recoveries
)

// Valid reports whether k is part of the closed display metadata list.
func (k SyncMetadataKey) Valid() bool {
	switch k {
	case SyncStatus, SyncRun, SyncAgent, SyncType, SyncModel, SyncNote, SyncPauseReason, SyncWakeAt, SyncCreated, SyncFinished, SyncRunStart, SyncJobID, SyncRestarts:
		return true
	}
	return false
}

// UnmarshalJSON implements json.Unmarshaler; its interface error contains *ValueError.
func (k *SyncMetadataKey) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, k, "sync metadata key").asError()
}

// SyncFilename is a permitted task artifact name, never a network-selected path.
type SyncFilename string

const (
	SyncActivity      SyncFilename = "activity.jsonl" // normalized activity tail
	SyncFinal         SyncFilename = "final.md"       // final agent message
	SyncResult        SyncFilename = "result.md"      // task result text
	syncFileBytes                  = 256 * 1024       // maximum UTF-8 bytes per artifact
	syncRequestBytes               = 2 * 1024 * 1024  // maximum encoded HTTP sync request bytes
	syncActivityLines              = 2000             // activity lines retained per task
	syncErrorLines                 = 500              // error log lines retained per run
	syncLogEntries                 = 500              // node log entries accepted per sync request
	syncLogLines                   = 20000            // maximum lines retained in each node log
	syncLogEntryBytes              = 64 * 1024        // maximum encoded bytes in one structured log entry
)

// syncRunFile matches only numbered prompt, question and error artifact basenames.
var syncRunFile = regexp.MustCompile(`^(prompt-[0-9]+\.md|questions-[0-9]+\.md|err-[0-9]+\.log)$`)

// Valid reports whether n belongs to the closed basename grammar.
func (n SyncFilename) Valid() bool {
	return n == SyncActivity || n == SyncFinal || n == SyncResult || syncRunFile.MatchString(string(n))
}

// UnmarshalJSON implements json.Unmarshaler; its interface error contains *ValueError.
func (n *SyncFilename) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, n, "sync filename").asError()
}

// leadSyncFile carries the bounded UTF-8 representation and its content digest.
type leadSyncFile struct {
	Name    string // untrusted basename, parsed explicitly for the InvalidFile cause
	Content string // exact bytes written to the mirror
	SHA256  string // lowercase SHA-256 of Content
	Cut     bool   // the node truncated the original artifact
}

// leadSyncBody updates one assigned job's task and appends bounded node logs.
type leadSyncBody struct {
	wireBytes  int                        // actual HTTP bytes, set only by the bounded decoder; zero for in-process calls
	Job        string                     // existing job identifier
	Assignment uint64                     // current assignment fence
	TaskID     string                     // node-local task recorded by the accepted start report
	Metadata   map[SyncMetadataKey]string // only changed display metadata
	Files      []leadSyncFile             // only changed task artifacts
	Logs       []json.RawMessage          // new structured node log records
}

// syncManifest retains accepted artifact digests and cut flags across lead restarts.
// This is new mirror-local state; jobs.json and the ordinary bus formats do not change.
type syncManifest struct {
	Files map[string]leadSyncFile // content cleared; only digests and cut indicators are stored
}

// decodeSyncBody enforces the larger sync envelope independently of other lead endpoints.
func decodeSyncBody(r *http.Request) (leadSyncBody, *LeadSyncError) {
	var body leadSyncBody // zero value is returned on malformed or oversized requests
	data, err := io.ReadAll(io.LimitReader(r.Body, syncRequestBytes+1))
	if err != nil {
		return body, opErr(LeadSyncMalformedBody, "cannot read sync: %v", err)
	}
	if len(data) > syncRequestBytes {
		return body, opErr(LeadSyncRequestTooLarge, "sync exceeds %d bytes", syncRequestBytes)
	}
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return body, opErr(LeadSyncMalformedBody, "sync must be one JSON object")
	}
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&body); err != nil {
		return body, opErr(LeadSyncMalformedBody, "invalid sync JSON: %v", err)
	}
	if dec.Decode(new(any)) != io.EOF {
		return body, opErr(LeadSyncMalformedBody, "sync must contain one JSON object")
	}
	body.wireBytes = len(data)
	return body, nil
}

// validateSync checks every byte and name before any mirror directory is created.
func validateSync(node string, b leadSyncBody) *LeadSyncError {
	if !validJobIdentifier(node) {
		return opErr(LeadSyncInvalidNode, "invalid node identifier")
	}
	if !validJobIdentifier(b.TaskID) {
		return opErr(LeadSyncInvalidTask, "invalid task identifier")
	}
	if b.wireBytes == 0 {
		// In-process callers have no raw envelope; transport callers were already bounded before decoding.
		data, err := json.Marshal(b)
		if err != nil {
			return opErr(LeadSyncMalformedBody, "invalid sync data: %v", err)
		}
		if len(data) > syncRequestBytes {
			return opErr(LeadSyncRequestTooLarge, "sync exceeds %d bytes", syncRequestBytes)
		}
	}
	for key, value := range b.Metadata {
		if _, err := parseEnum[SyncMetadataKey](string(key), "sync metadata key"); err != nil {
			return fromValue(LeadSyncInvalidMetadata, err)
		}
		if !utf8.ValidString(value) || strings.ContainsRune(value, 0) {
			return opErr(LeadSyncInvalidMetadata, "invalid value for %s", key)
		}
	}
	seen := make(map[string]bool)
	for _, file := range b.Files {
		name, err := parseEnum[SyncFilename](file.Name, "sync filename")
		if err != nil || seen[file.Name] {
			return opErr(LeadSyncInvalidFile, "invalid or repeated artifact %q", file.Name)
		}
		seen[file.Name] = true
		lines := strings.Count(file.Content, "\n")
		if file.Content != "" && !strings.HasSuffix(file.Content, "\n") {
			lines++
		}
		if len(file.Content) > syncFileBytes || name == SyncActivity && lines > syncActivityLines || strings.HasPrefix(file.Name, "err-") && lines > syncErrorLines {
			return opErr(LeadSyncFileTooLarge, "artifact %q exceeds its byte or line limit", file.Name)
		}
		sum := sha256.Sum256([]byte(file.Content))
		if !utf8.ValidString(file.Content) || file.SHA256 != hex.EncodeToString(sum[:]) {
			return opErr(LeadSyncInvalidDigest, "artifact %q does not match its UTF-8 SHA-256", file.Name)
		}
	}
	if len(b.Logs) > syncLogEntries {
		return opErr(LeadSyncInvalidLogs, "too many log entries")
	}
	for _, entry := range b.Logs {
		var fields map[string]json.RawMessage // preserve arbitrary structured context without accepting paths
		if len(entry) > syncLogEntryBytes || json.Unmarshal(entry, &fields) != nil || fields == nil {
			return opErr(LeadSyncInvalidLogs, "log entry must be a bounded JSON object")
		}
		var level, task string // only these fields determine whether a record is in scope
		if err := json.Unmarshal(fields["level"], &level); err != nil {
			return opErr(LeadSyncInvalidLogs, "log level must be a string")
		}
		if value, ok := fields["task"]; ok {
			if err := json.Unmarshal(value, &task); err != nil {
				return opErr(LeadSyncInvalidLogs, "log task must be a string")
			}
		}

	}
	return nil
}

// sync stores validated data using the registry's existing lock and authorization.
func (a *leadAPI) sync(node string, body leadSyncBody) *LeadSyncError {
	if err := validateSync(node, body); err != nil {
		return err
	}
	return transactJobs(a.bus, LeadSyncStorage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *LeadSyncError {
		job, ok := r.jobs[body.Job]
		if !ok {
			return opErr(LeadSyncNoSuchJob, "job does not exist")
		}
		if job.Node != node {
			return opErr(LeadSyncUnauthorized, "job is not assigned to this node")
		}
		if job.Assignment != body.Assignment || body.Assignment == 0 {
			return opErr(LeadSyncStaleAssignment, "assignment is no longer current")
		}
		if job.TaskID != body.TaskID {
			return opErr(LeadSyncInvalidTask, "task differs from the job's accepted task")
		}
		if id, ok := body.Metadata[SyncJobID]; ok && id != body.Job {
			return opErr(LeadSyncInvalidMetadata, "JOB_ID differs from the assigned job")
		}
		for _, raw := range body.Logs {
			var entry struct {
				Level string `json:"level"` // record severity
				Task  string `json:"task"`  // node-local task identity
			}
			if err := json.Unmarshal(raw, &entry); err != nil {
				return opErr(LeadSyncInvalidLogs, "invalid structured log: %v", err)
			}
			if entry.Level == "warn" || entry.Level == "error" {
				continue
			}
			owned := false
			for _, candidate := range r.jobs {
				if candidate.Node == node && candidate.TaskID != "" && candidate.TaskID == entry.Task {
					owned = true
					break
				}
			}
			if !owned {
				return opErr(LeadSyncInvalidLogs, "log entry is outside this node's job tasks")
			}
		}
		if err := a.bus.storeSync(node, body); err != nil {
			return fromStorage(LeadSyncStorage, err)
		}
		return nil
	})
}

// readSyncJSON loads optional mirror-local JSON without silently resetting corrupt data.
func readSyncJSON(path string, out any) *StorageError {
	data, err := os.ReadFile(path) // #nosec G304 -- path is constructed internally under the bus mirror from validated identifiers.
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return storageErr(err, "cannot read %s", path)
	}
	if err := json.Unmarshal(data, out); err != nil {
		return storageErr(err, "cannot decode %s", path)
	}
	return nil
}

// writeSyncFile atomically replaces one mirror-owned file while bus.lock is held.
func writeSyncFile(path string, data []byte) *StorageError {
	if err := os.WriteFile(path+".tmp", data, 0600); err != nil {
		return storageErr(err, "cannot write %s", path)
	}
	if err := os.Rename(path+".tmp", path); err != nil {
		return storageErr(err, "cannot replace %s", path)
	}
	return nil
}

// writeSyncJSON encodes mirror state before its atomic replacement.
func writeSyncJSON(path string, value any) *StorageError {
	data, err := json.Marshal(value)
	if err != nil {
		return storageErr(err, "cannot encode %s", path)
	}
	return writeSyncFile(path, data)
}

// storeSync merges only accepted changes; retries repair partially written artifacts after storage failures.
func (b *Bus) storeSync(node string, body leadSyncBody) *StorageError {
	dir := filepath.Join(b.dir, "remote", node, body.TaskID)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return storageErr(err, "cannot create mirror")
	}
	meta := make(map[SyncMetadataKey]string)
	if err := readSyncJSON(filepath.Join(dir, "metadata.json"), &meta); err != nil {
		return err
	}
	for key, value := range body.Metadata {
		meta[key] = value
	}
	manifest := syncManifest{Files: make(map[string]leadSyncFile)}
	if err := readSyncJSON(filepath.Join(dir, "sync.json"), &manifest); err != nil {
		return err
	}
	if manifest.Files == nil {
		manifest.Files = make(map[string]leadSyncFile)
	}
	for _, file := range body.Files {
		if err := writeSyncFile(filepath.Join(dir, file.Name), []byte(file.Content)); err != nil {
			return err
		}
		file.Content = ""
		manifest.Files[file.Name] = file
	}
	if err := writeSyncJSON(filepath.Join(dir, "metadata.json"), meta); err != nil {
		return err
	}
	if len(body.Logs) > 0 {
		if err := b.storeSyncLogs(node, body.Logs); err != nil {
			return err
		}
	}
	return writeSyncJSON(filepath.Join(dir, "sync.json"), manifest)
}

// syncLogState atomically commits a bounded log tail together with its last accepted batch identity.
// logs.jsonl is a recoverable materialized view, so a lost response or a failed view write cannot append twice.
type syncLogState struct {
	Digest  string            // SHA-256 of the last nonempty batch from this node
	Entries []json.RawMessage // at most syncLogLines compact JSON records
}

// storeSyncLogs keeps the bounded tail and compacts each JSON record onto one line.
func (b *Bus) storeSyncLogs(node string, entries []json.RawMessage) *StorageError {
	dir := filepath.Join(b.dir, "remote", node)
	path := filepath.Join(dir, "logs.jsonl")
	statePath := filepath.Join(dir, "logs-state.json")
	state := syncLogState{}
	if err := readSyncJSON(statePath, &state); err != nil {
		return err
	}
	if state.Entries == nil {
		data, err := os.ReadFile(path) // #nosec G304 -- path is constructed internally under the bus mirror from validated identifiers.
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return storageErr(err, "cannot read node logs")
		}
		for _, line := range bytes.Split(bytes.TrimSpace(data), []byte("\n")) {
			if len(line) > 0 {
				state.Entries = append(state.Entries, json.RawMessage(line))
			}
		}
	}
	encoded, err := json.Marshal(entries)
	if err != nil {
		return storageErr(err, "cannot encode log batch")
	}
	digest := fmt.Sprintf("%x", sha256.Sum256(encoded))
	if state.Digest != digest {
		state.Entries = append(state.Entries, entries...)
		if len(state.Entries) > syncLogLines {
			state.Entries = state.Entries[len(state.Entries)-syncLogLines:]
		}
		state.Digest = digest
		if err := writeSyncJSON(statePath, state); err != nil {
			return err
		}
	}
	var out bytes.Buffer // materialize only the committed tail; retries repair this view
	for _, entry := range state.Entries {
		if err := json.Compact(&out, entry); err != nil {
			return storageErr(err, "cannot compact node log")
		}
		if err := out.WriteByte('\n'); err != nil {
			return storageErr(err, "cannot delimit node log")
		}
	}
	return writeSyncFile(path, out.Bytes())
}

// pruneMirrors removes task mirrors no longer owned by a retained job; node log tails remain bounded independently.
// It runs under bus.lock after successful registry writes, and retries cleanup on every later transaction.
func (b *Bus) pruneMirrors(r *leadRegistry) *StorageError {
	root := filepath.Join(b.dir, "remote")
	nodes, err := os.ReadDir(root)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return storageErr(err, "cannot list remote mirrors")
	}
	kept := make(map[string]bool)
	for _, job := range r.jobs {
		kept[job.Node+"/"+job.TaskID] = true
	}
	for _, node := range nodes {
		if !node.IsDir() || !validJobIdentifier(node.Name()) {
			continue
		}
		dir := filepath.Join(root, node.Name())
		tasks, err := os.ReadDir(dir)
		if err != nil {
			return storageErr(err, "cannot list node mirrors")
		}
		for _, task := range tasks {
			if task.IsDir() && validJobIdentifier(task.Name()) && !kept[node.Name()+"/"+task.Name()] {
				if err := os.RemoveAll(filepath.Join(dir, task.Name())); err != nil {
					return storageErr(err, "cannot remove expired mirror")
				}
			}
		}
	}
	return nil
}
