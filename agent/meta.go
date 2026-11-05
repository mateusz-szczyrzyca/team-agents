package main

import (
	"errors"
	"os"
	"strings"
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
