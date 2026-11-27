// Repository findings and their append-only journal.
package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Finding is an open note tied to repository paths.
type Finding struct {
	ID     string      `json:"id"`     // Stable F-number.
	TS     Timestamp   `json:"ts"`     // Addition time.
	Author string      `json:"author"` // Agent that added the note.
	Task   string      `json:"task"`   // Task that added the note.
	Kind   FindingKind `json:"kind"`   // Category of the note.
	Paths  []string    `json:"paths"`  // Normalized repository paths.
	Text   string      `json:"text"`   // Note for later agents.
}

// findingOp distinguishes journal additions from resolutions.
type findingOp string

const (
	findingOpAdd     findingOp = "add"     // Introduces the finding carried by the record.
	findingOpResolve findingOp = "resolve" // Closes the named finding.
)

// Valid reports whether op is a journal operation.
func (op findingOp) Valid() bool { return op == findingOpAdd || op == findingOpResolve }

// UnmarshalJSON implements json.Unmarshaler; failures contain a *ValueError.
func (op *findingOp) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, op, "finding operation").asError()
}

// findingRecord is the read representation of either journal operation.
type findingRecord struct {
	Op      findingOp `json:"op"` // Operation to replay.
	Finding           // Addition fields; resolutions use only identity and attribution.
	Reason  string    `json:"reason,omitempty"` // Why a finding was resolved.
}

// findingResolution is the write representation without addition-only fields.
type findingResolution struct {
	Op     findingOp `json:"op"`     // Always a resolution.
	ID     string    `json:"id"`     // Finding being closed.
	TS     Timestamp `json:"ts"`     // Resolution time.
	Author string    `json:"author"` // Agent closing the finding.
	Task   string    `json:"task"`   // Task closing the finding.
	Reason string    `json:"reason"` // Explanation of the resolution.
}

// findingJournal is the replayed repository journal.
type findingJournal struct {
	added    []Finding       // Additions in file order.
	resolved map[string]bool // IDs with a resolution record.
	highest  uint64          // Largest allocated ID, including resolved findings.
}

// readFindings replays the journal while the caller holds the bus lock. Missing journals are empty.
func (b *Bus) readFindings(repo string) (journal findingJournal, serr *StorageError) {
	journal.resolved = map[string]bool{}
	if name := safeName(repo); name == "." || name == ".." {
		return journal, storageErr(valueErr("invalid repository name %q", repo), "cannot read findings")
	}
	file := filepath.Join(b.notebookDir(repo), "findings.jsonl")
	f, err := os.Open(file) //nolint:gosec // G304: fixed findings file under the configured bus notebook directory.
	if errors.Is(err, os.ErrNotExist) {
		return journal, nil
	}
	if err != nil {
		return journal, storageErr(err, "cannot read findings of %s", repo)
	}
	defer func() {
		if err := f.Close(); err != nil {
			if serr == nil {
				serr = storageErr(err, "cannot close findings of %s", repo)
			} else {
				// Preserve the primary read failure while reporting the cleanup failure.
				warnSideEffect("close findings journal", storageErr(err, "cannot close findings of %s", repo))
			}
		}
	}()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		var record findingRecord
		if err := json.Unmarshal(sc.Bytes(), &record); err != nil {
			return journal, storageErr(err, "cannot decode findings of %s", repo)
		}
		n, err := strconv.ParseUint(strings.TrimPrefix(record.ID, "F-"), 10, 64)
		if err != nil {
			return journal, storageErr(err, "invalid finding id %q", record.ID)
		}
		if !strings.HasPrefix(record.ID, "F-") || n == 0 {
			return journal, storageErr(valueErr("invalid finding id %q", record.ID), "cannot replay findings of %s", repo)
		}
		if n > journal.highest {
			journal.highest = n
		}
		switch record.Op {
		case findingOpAdd:
			journal.added = append(journal.added, record.Finding)
		case findingOpResolve:
			journal.resolved[record.ID] = true
		default:
			return journal, storageErr(valueErr("unknown finding operation %q", record.Op), "cannot replay findings of %s", repo)
		}
	}
	if err := sc.Err(); err != nil {
		return journal, storageErr(err, "cannot scan findings of %s", repo)
	}
	return journal, nil
}

// appendFinding writes one JSON record without rewriting earlier records; caller holds the bus lock.
func (b *Bus) appendFinding(repo string, record any) *StorageError {
	data, err := json.Marshal(record)
	if err != nil {
		return storageErr(err, "cannot encode finding of %s", repo)
	}
	return appendFile(filepath.Join(b.notebookDir(repo), "findings.jsonl"), string(data)+"\n")
}

// normalizeFindingPaths rejects ambiguous paths before cleaning relative repository paths.
func normalizeFindingPaths(paths []string) ([]string, *ValueError) {
	out := make([]string, 0, len(paths))
	for _, p := range paths {
		if p == "" || strings.HasPrefix(p, "/") || strings.ContainsFunc(p, unicode.IsSpace) || utf8.RuneCountInString(p) > 200 {
			return nil, valueErr("invalid repository path %q", p)
		}
		for _, part := range strings.Split(p, "/") {
			if part == ".." {
				return nil, valueErr("parent component in repository path %q", p)
			}
		}
		clean := path.Clean(p)
		if clean == "." {
			return nil, valueErr("repository path %q names no file or directory", p)
		}
		out = append(out, clean)
	}
	return out, nil
}

// findingAdd stores a finding, allocating its ID under the same lock as the append.
func (s *Server) findingAdd(kind string, paths []string, text string) (string, *FindingAddError) {
	text = strings.TrimSpace(text)
	if text == "" {
		return "", opErr(FindingAddMissingText, "%s", missing("text"))
	}
	if utf8.RuneCountInString(text) > 600 {
		return "", opErr(FindingAddTextTooLong, "text exceeds 600 characters")
	}
	k, verr := parseEnum[FindingKind](kind, "finding kind")
	if verr != nil {
		return "", fromValue(FindingAddInvalidKind, verr)
	}
	if len(paths) == 0 && k != FindingEnvironment {
		return "", opErr(FindingAddMissingPaths, "%s requires at least one path", k)
	}
	if len(paths) > 10 {
		return "", opErr(FindingAddTooManyPaths, "at most ten paths are allowed")
	}
	normalized, verr := normalizeFindingPaths(paths)
	if verr != nil {
		return "", fromValue(FindingAddInvalidPath, verr)
	}
	var id string
	err := transact(s.bus, FindingAddStorage, func(st *State) *FindingAddError {
		s.touch(st)
		journal, serr := s.bus.readFindings(s.info.Repo)
		if serr != nil {
			return fromStorage(FindingAddStorage, serr)
		}
		count := 0
		for _, f := range journal.added {
			if f.Task == s.info.Task && !journal.resolved[f.ID] {
				count++
			}
		}
		if count >= 10 {
			return opErr(FindingAddTooManyOpen, "resolve an open finding before adding another (maximum ten per task)")
		}
		if journal.highest == math.MaxUint64 {
			return fromStorage(FindingAddStorage, storageErr(valueErr("finding ids exhausted"), "cannot allocate finding id"))
		}
		id = fmt.Sprintf("F-%d", journal.highest+1)
		finding := Finding{ID: id, TS: Now(), Author: s.me, Task: s.info.Task, Kind: k, Paths: normalized, Text: text}
		if serr := s.bus.appendFinding(s.info.Repo, findingRecord{Op: findingOpAdd, Finding: finding}); serr != nil {
			return fromStorage(FindingAddStorage, serr)
		}
		logInfo(LogFindingAdded, "repo", s.info.Repo, "id", id, "author", s.me, "task", s.info.Task)
		if s.team != "" {
			_, serr := s.bus.post(st, s.me, teamRecipient(s.team), EventFindingAdded{ID: id, KindOfFinding: k, Paths: normalized, TextOfFinding: text})
			return fromStorage(FindingAddStorage, serr)
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return "ADDED " + id, nil
}

// findingResolve closes any task's finding; repeat resolutions leave the journal unchanged.
func (s *Server) findingResolve(id, reason string) (string, *FindingResolveError) {
	id, reason = strings.TrimSpace(id), strings.TrimSpace(reason)
	if id == "" {
		return "", opErr(FindingResolveMissingId, "%s", missing("id"))
	}
	if reason == "" {
		return "", opErr(FindingResolveMissingReason, "%s", missing("reason"))
	}
	if utf8.RuneCountInString(reason) > 300 {
		return "", opErr(FindingResolveReasonTooLong, "reason exceeds 300 characters")
	}
	var out string
	err := transact(s.bus, FindingResolveStorage, func(st *State) *FindingResolveError {
		s.touch(st)
		journal, serr := s.bus.readFindings(s.info.Repo)
		if serr != nil {
			return fromStorage(FindingResolveStorage, serr)
		}
		found := false
		for _, f := range journal.added {
			if f.ID == id {
				found = true
				break
			}
		}
		if !found {
			return opErr(FindingResolveUnknownFinding, "finding %q does not exist", id)
		}
		if journal.resolved[id] {
			out = "ALREADY RESOLVED " + id
			return nil
		}
		record := findingResolution{Op: findingOpResolve, ID: id, TS: Now(), Author: s.me, Task: s.info.Task, Reason: reason}
		if serr := s.bus.appendFinding(s.info.Repo, record); serr != nil {
			return fromStorage(FindingResolveStorage, serr)
		}
		logInfo(LogFindingResolved, "repo", s.info.Repo, "id", id, "author", s.me, "task", s.info.Task)
		if s.team != "" {
			if _, serr := s.bus.post(st, s.me, teamRecipient(s.team), EventFindingResolved{ID: id, Reason: reason}); serr != nil {
				return fromStorage(FindingResolveStorage, serr)
			}
		}
		out = "RESOLVED " + id
		return nil
	})
	if err != nil {
		return "", err
	}
	return out, nil
}

// findings lists open findings under the bus lock, oldest first.
func (b *Bus) findings(repo string) ([]Finding, *FindingsError) {
	if strings.TrimSpace(repo) == "" {
		return nil, opErr(FindingsMissingRepo, "%s", missing("repo"))
	}
	out := []Finding{}
	serr := b.withLock(func() *StorageError {
		journal, serr := b.readFindings(repo)
		if serr != nil {
			return serr
		}
		for _, f := range journal.added {
			if !journal.resolved[f.ID] {
				out = append(out, f)
			}
		}
		sort.SliceStable(out, func(i, j int) bool { return out[i].TS.Before(out[j].TS.Time) })
		return nil
	})
	if serr != nil {
		return nil, fromStorage(FindingsStorage, serr)
	}
	return out, nil
}

// cmdFindings prints the open repository findings as JSON for the daemon.
func cmdFindings(args []string) {
	fs := newFlagSet("findings")
	dir := fs.String("dir", defaultDir(), "state directory")
	repo := fs.String("repo", "", "repository name")
	fs.Bool("json", false, "print JSON (also the default)")
	parseFlags(fs, args)
	findings, err := (&Bus{dir: *dir}).findings(*repo)
	exitOn(err.asError())
	exitOn(fromStorage(FindingsStorage, printJSON(findings)).asError())
}
