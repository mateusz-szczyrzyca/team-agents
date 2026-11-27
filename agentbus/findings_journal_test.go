package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
)

func TestBusFindingStorageCauses(t *testing.T) {
	b := newBus(t)
	b.dir = filepath.Join(b.dir, "not-a-directory")
	if err := os.WriteFile(b.dir, []byte("file"), 0o600); err != nil {
		t.Fatal(err)
	}
	s := server(b, "a")
	failsWith(t, FindingAddStorage)(s.findingAdd("trap", []string{"src/a"}, "note"))
	failsWith(t, FindingResolveStorage)(s.findingResolve("F-1", "fixed"))
	_, err := b.findings("repo")
	failsWith(t, FindingsStorage)("", err)
}

func TestBusFindingUnicodeBoundariesAndAppendOnlySchema(t *testing.T) {
	b := newBus(t)
	s := server(b, "a")
	text := strings.Repeat("é", 600)
	path := strings.Repeat("a", 200)
	chk(t).ok(toolResult(s.findingAdd("flaky_test", []string{path}, text)))
	file := filepath.Join(b.notebookDir("repo"), "findings.jsonl")
	before, err := os.ReadFile(file) // #nosec G304 -- fixed findings journal in this test-owned temporary bus directory.
	if err != nil {
		t.Fatal(err)
	}
	chk(t).ok(toolResult(server(b, "b").findingResolve("F-1", strings.Repeat("é", 300))))
	after, err := os.ReadFile(file) // #nosec G304 -- fixed findings journal in this test-owned temporary bus directory.
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(string(after), string(before)) {
		t.Fatal("existing addition was rewritten")
	}
	lines := strings.Split(strings.TrimSpace(string(after)), "\n")
	if len(lines) != 2 {
		t.Fatalf("got %d lines", len(lines))
	}
	for i, keys := range [][]string{{"op", "id", "ts", "author", "task", "kind", "paths", "text"}, {"op", "id", "ts", "author", "task", "reason"}} {
		var record map[string]json.RawMessage
		if err := json.Unmarshal([]byte(lines[i]), &record); err != nil {
			t.Fatal(err)
		}
		if len(record) != len(keys) {
			t.Fatalf("got fields %v", record)
		}
		for _, key := range keys {
			if _, ok := record[key]; !ok {
				t.Fatalf("missing %s", key)
			}
		}
	}
	got, ferr := b.findings("repo")
	if ferr != nil {
		t.Fatal(ferr)
	}
	if !reflect.DeepEqual(got, []Finding{}) {
		t.Fatalf("got %#v", got)
	}
	chk(t).ok(toolResult(s.findingResolve("F-1", "repeat")))
	repeated, err := os.ReadFile(file) // #nosec G304 -- fixed findings journal in this test-owned temporary bus directory.
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != string(repeated) {
		t.Fatal("repeat resolution appended a record")
	}
}

func TestBusFindingConcurrentAddsKeepIDsAndTaskLimit(t *testing.T) {
	b := newBus(t)
	var wg sync.WaitGroup
	results := make(chan string, 11)
	failures := make(chan *FindingAddError, 11)
	for range 11 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			out, err := server(b, "a").findingAdd("trap", []string{"src/a"}, "note")
			if err != nil {
				failures <- err
			} else {
				results <- out
			}
		}()
	}
	wg.Wait()
	close(results)
	close(failures)
	seen := map[string]bool{}
	for out := range results {
		if seen[out] {
			t.Fatalf("duplicate %q", out)
		}
		seen[out] = true
	}
	if len(seen) != 10 || len(failures) != 1 {
		t.Fatalf("got %d successes and %d failures", len(seen), len(failures))
	}
	for err := range failures {
		failsWith(t, FindingAddTooManyOpen)("", err)
	}
}

func TestBusFindingCorruptJournalIsStorageFailure(t *testing.T) {
	for _, data := range []string{"not json\n", `{"op":"add","id":"F-bad"}`, `{"op":"other","id":"F-1"}`, `{"op":"add","id":"F-1","kind":"unknown"}`} {
		b := newBus(t)
		if err := os.MkdirAll(b.notebookDir("repo"), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(b.notebookDir("repo"), "findings.jsonl"), []byte(data), 0o600); err != nil {
			t.Fatal(err)
		}
		s := server(b, "a")
		failsWith(t, FindingAddStorage)(s.findingAdd("trap", []string{"src/a"}, "note"))
		failsWith(t, FindingResolveStorage)(s.findingResolve("F-1", "fixed"))
		_, err := b.findings("repo")
		failsWith(t, FindingsStorage)("", err)
	}
}

func TestBusFindingRepositoryCannotEscapeNotebookDirectory(t *testing.T) {
	for _, repo := range []string{".", ".."} {
		b := newBus(t)
		s := server(b, "a")
		s.info.Repo = repo
		failsWith(t, FindingAddStorage)(s.findingAdd("trap", []string{"src/a"}, "note"))
		failsWith(t, FindingResolveStorage)(s.findingResolve("F-1", "fixed"))
		_, err := b.findings(repo)
		failsWith(t, FindingsStorage)("", err)
	}
}
