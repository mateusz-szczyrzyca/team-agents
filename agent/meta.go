package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// MetaLineKind distinguishes decoded data from records requiring shell evaluation.
type MetaLineKind uint8

const (
	MetaBlank      MetaLineKind = iota // MetaBlank retains whitespace-only records.
	MetaComment                        // MetaComment retains comments without evaluating them.
	MetaAssignment                     // MetaAssignment contains a literal scalar assignment.
	MetaShell                          // MetaShell retains unsupported shell syntax verbatim.
)

// Valid reports whether the record classification is known.
func (k MetaLineKind) Valid() bool { return k <= MetaShell }

// MetaLine retains physical bytes as well as the decoded assignment when available.
type MetaLine struct {
	Kind    MetaLineKind // classification; shell records never execute
	Raw     string       // original bytes, including the terminating newline if present
	Key     string       // recognized assignment prefix, including opaque values; empty otherwise
	Value   string       // decoded literal value, including explicitly empty values
	Encoded string       // original assignment word for consumers comparing encoded text
}

// MetaFile owns ordered records; unknown names and shell records are preserved.
type MetaFile struct {
	Lines []MetaLine // source order, including duplicate assignments
}

// parseMeta classifies literal records without executing metadata as a program.
func parseMeta(data []byte) MetaFile {
	var file MetaFile
	for remaining := string(data); remaining != ""; {
		end := recordEnd(remaining)
		if end < len(remaining) {
			end++
		}
		file.Lines = append(file.Lines, classifyMeta(remaining[:end]))
		remaining = remaining[end:]
	}
	return file
}

// Lookup returns the last literal assignment unless a later keyed shell record hides it.
// A false result means absent or opaque; a literal empty value still returns true.
func (m MetaFile) Lookup(key string) (string, bool) {
	for i := len(m.Lines) - 1; i >= 0; i-- {
		line := m.Lines[i]
		if line.Key == key {
			return line.Value, line.Kind == MetaAssignment
		}
	}
	return "", false
}

// Bytes reproduces the original file, including unsupported records and final newline state.
func (m MetaFile) Bytes() []byte {
	var out strings.Builder
	for _, line := range m.Lines {
		out.WriteString(line.Raw)
	}
	return []byte(out.String())
}

// readTaskMeta reads an existing file without inheriting any caller variables.
// Non-regular paths deliberately report Storage rather than Bash meta_load's NotFound.
func readTaskMeta(tasksDir, id string) (MetaFile, *ReadTaskMetaError) {
	path := tasksDir + "/" + id + "/meta.env"
	data, err := os.ReadFile(path) // #nosec G304 -- local task paths deliberately preserve Bash path-like references; no network request supplies this path
	if errors.Is(err, os.ErrNotExist) {
		return MetaFile{}, opErr(ReadTaskMetaNotFound, "no such task: %s", id)
	}
	if err != nil {
		return MetaFile{}, fromStorage(ReadTaskMetaStorage, storageErr(err, "cannot read %s", path))
	}
	return parseMeta(data), nil
}

// jobTransition captures one status change record appended to job_transitions.jsonl.
type jobTransition struct {
	State string `json:"state"` // task state name
	At    string `json:"at"`    // ISO-8601 UTC timestamp
}

// setTaskMeta writes a key=value pair to the task metadata file under lock.
// If runID is provided and does not match the current RUN value, returns nil (stale-run fence).
func setTaskMeta(tasksDir, taskID, key, value, runID string) (retErr *SetTaskMetaError) {
	f, lockErr := lockTaskMeta(tasksDir, taskID, 0)
	if lockErr != nil {
		return lockErr
	}
	defer func() {
		if unlockErr := unlockTaskMeta(f); unlockErr != nil && retErr == nil {
			retErr = fromStorage(SetTaskMetaStorage, unlockErr)
		}
	}()

	metaPath := filepath.Join(tasksDir, taskID, "meta.env")
	data, readErr := os.ReadFile(metaPath) // #nosec G304,G703 -- local task path under isolated tasks directory
	if readErr != nil {
		if errors.Is(readErr, os.ErrNotExist) {
			return opErr(SetTaskMetaNotFound, "no such task: %s", taskID)
		}
		return fromStorage(SetTaskMetaStorage, storageErr(readErr, "read metadata %s", metaPath))
	}

	m := parseMeta(data)

	if runID != "" {
		currentRun, _ := m.Lookup("RUN")
		if currentRun != runID {
			return nil
		}
	}

	newLine, verr := formatMetaAssignment(key, value)
	if verr != nil {
		return fromValue(SetTaskMetaInvalidArgument, verr)
	}

	var buf bytes.Buffer
	for _, line := range m.Lines {
		if line.Key == key {
			continue
		}
		buf.WriteString(line.Raw)
	}
	if buf.Len() > 0 && !strings.HasSuffix(buf.String(), "\n") {
		buf.WriteByte('\n')
	}
	buf.WriteString(newLine)

	tmp, err := os.CreateTemp(filepath.Join(tasksDir, taskID), "meta.env.tmp.*")
	if err != nil {
		return fromStorage(SetTaskMetaStorage, storageErr(err, "create temp metadata in %s", filepath.Join(tasksDir, taskID)))
	}
	tmpName := tmp.Name()

	if _, err := tmp.Write(buf.Bytes()); err != nil {
		if closeErr := tmp.Close(); closeErr != nil {
			err = errors.Join(err, closeErr)
		}
		if removeErr := os.Remove(tmpName); removeErr != nil { // #nosec G703 -- tmpName is from os.CreateTemp
			err = errors.Join(err, removeErr)
		}
		return fromStorage(SetTaskMetaStorage, storageErr(err, "write temp metadata %s", tmpName))
	}
	if err := tmp.Close(); err != nil {
		if removeErr := os.Remove(tmpName); removeErr != nil { // #nosec G703 -- tmpName is from os.CreateTemp
			err = errors.Join(err, removeErr)
		}
		return fromStorage(SetTaskMetaStorage, storageErr(err, "close temp metadata %s", tmpName))
	}
	if err := os.Rename(tmpName, metaPath); err != nil { // #nosec G703 -- tmpName is from os.CreateTemp, metaPath is constructed from trusted task coordinates
		if removeErr := os.Remove(tmpName); removeErr != nil { // #nosec G703 -- tmpName is from os.CreateTemp
			err = errors.Join(err, removeErr)
		}
		return fromStorage(SetTaskMetaStorage, storageErr(err, "rename %s to %s", tmpName, metaPath))
	}

	if key == "STATUS" {
		if jid, ok := m.Lookup("JOB_ID"); ok && jid != "" {
			now := time.Now().UTC().Format(time.RFC3339Nano)
			recordBytes, err := json.Marshal(jobTransition{State: value, At: now})
			if err != nil {
				return fromStorage(SetTaskMetaStorage, storageErr(err, "marshal job transition"))
			}
			recordBytes = append(recordBytes, '\n')

			journalPath := filepath.Join(tasksDir, taskID, "job_transitions.jsonl")
			jf, err := os.OpenFile(journalPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600) // #nosec G304,G703 -- local task path under isolated tasks directory
			if err != nil {
				return fromStorage(SetTaskMetaStorage, storageErr(err, "open journal %s", journalPath))
			}
			if _, err := jf.Write(recordBytes); err != nil {
				if closeErr := jf.Close(); closeErr != nil {
					err = errors.Join(err, closeErr)
				}
				return fromStorage(SetTaskMetaStorage, storageErr(err, "append journal %s", journalPath))
			}
			if err := jf.Close(); err != nil {
				return fromStorage(SetTaskMetaStorage, storageErr(err, "close journal %s", journalPath))
			}
		}
	}

	return nil
}
