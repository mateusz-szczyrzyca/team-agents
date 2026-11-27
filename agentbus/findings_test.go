package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestFindingAddNumbersFromOne(t *testing.T) {
	s := server(newBus(t), "a")
	if got := chk(t).ok(toolResult(s.findingAdd("trap", []string{"crates/a"}, "watch this"))); got != "ADDED F-1" {
		t.Fatalf("got %q", got)
	}
}

func TestFindingAddNumbersAfterHighest(t *testing.T) {
	b := newBus(t)
	dir := b.notebookDir("repo")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	data := ""
	for _, id := range []string{"F-1", "F-7"} {
		data += fmt.Sprintf(`{"op":"add","id":%q,"ts":"2026-11-27T10:00:00Z","author":"b","task":"other","kind":"trap","paths":["src/a"],"text":"note"}`+"\n", id)
	}
	if err := os.WriteFile(filepath.Join(dir, "findings.jsonl"), []byte(data), 0o600); err != nil {
		t.Fatal(err)
	}
	if got := chk(t).ok(toolResult(server(b, "a").findingAdd("trap", []string{"src/a"}, "next"))); got != "ADDED F-8" {
		t.Fatalf("got %q", got)
	}
}

func TestFindingPathsAreNormalized(t *testing.T) {
	b := newBus(t)
	chk(t).ok(toolResult(server(b, "a").findingAdd("trap", []string{"./crates/a/"}, "note")))
	got, err := b.findings("repo")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || !reflect.DeepEqual(got[0].Paths, []string{"crates/a"}) {
		t.Fatalf("got %#v", got)
	}
}

func TestEnvironmentFindingWithoutPaths(t *testing.T) {
	b := newBus(t)
	chk(t).ok(toolResult(server(b, "a").findingAdd("environment", nil, "tool note")))
	got, err := b.findings("repo")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Kind != FindingEnvironment || !reflect.DeepEqual(got[0].Paths, []string{}) {
		t.Fatalf("got %#v", got)
	}
}

func TestFindingResolveTwiceIsAResult(t *testing.T) {
	b := newBus(t)
	chk(t).ok(toolResult(server(b, "a").findingAdd("trap", []string{"src/a"}, "note")))
	s := server(b, "b")
	for _, want := range []string{"RESOLVED F-1", "ALREADY RESOLVED F-1"} {
		if got := chk(t).ok(toolResult(s.findingResolve("F-1", "fixed"))); got != want {
			t.Fatalf("got %q want %q", got, want)
		}
	}
}

func TestFindingsListsOnlyOpen(t *testing.T) {
	b := newBus(t)
	s := server(b, "a")
	for _, text := range []string{"first", "second", "third"} {
		chk(t).ok(toolResult(s.findingAdd("trap", []string{"src/a"}, text)))
	}
	chk(t).ok(toolResult(s.findingResolve("F-2", "fixed")))
	got, err := b.findings("repo")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].ID != "F-1" || got[1].ID != "F-3" || got[0].Author != "a" || got[0].Task != "task-a" || got[0].Text != "first" || got[0].TS.IsZero() {
		t.Fatalf("got %#v", got)
	}
}

func TestFindingsMissingFileIsEmpty(t *testing.T) {
	got, err := newBus(t).findings("repo")
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, []Finding{}) {
		t.Fatalf("got %#v", got)
	}
}

func TestFindingKindRejectsUnknown(t *testing.T) {
	var kind FindingKind
	err := json.Unmarshal([]byte(`"unknown"`), &kind)
	var value *ValueError
	if !errors.As(err, &value) {
		t.Fatalf("got %v", err)
	}
	for _, raw := range []string{"flaky_test", "trap", "environment"} {
		if err := json.Unmarshal([]byte(fmt.Sprintf("%q", raw)), &kind); err != nil {
			t.Fatal(err)
		}
		if string(kind) != raw {
			t.Fatalf("got %q", kind)
		}
	}
}

func TestFindingAddValidation(t *testing.T) {
	s := server(newBus(t), "a")
	failsWith(t, FindingAddMissingText)(s.findingAdd("trap", []string{"src/a"}, "  "))
	failsWith(t, FindingAddTextTooLong)(s.findingAdd("trap", []string{"src/a"}, strings.Repeat("é", 601)))
	failsWith(t, FindingAddInvalidKind)(s.findingAdd("other", nil, "note"))
	for _, kind := range []string{"trap", "flaky_test"} {
		failsWith(t, FindingAddMissingPaths)(s.findingAdd(kind, nil, "note"))
	}
	failsWith(t, FindingAddTooManyPaths)(s.findingAdd("environment", make([]string, 11), "note"))
	for _, path := range []string{"", "/src/a", "src/../a", "src/a b", strings.Repeat("a", 201), ".", "./", "src/.."} {
		failsWith(t, FindingAddInvalidPath)(s.findingAdd("trap", []string{path}, "note"))
	}
}

func TestFindingAddOpenLimitIsPerTaskAndReleasedByResolve(t *testing.T) {
	b := newBus(t)
	s := server(b, "a")
	for range 10 {
		chk(t).ok(toolResult(s.findingAdd("trap", []string{"src/a"}, "note")))
	}
	failsWith(t, FindingAddTooManyOpen)(s.findingAdd("trap", []string{"src/a"}, "note"))
	chk(t).ok(toolResult(server(b, "b").findingAdd("trap", []string{"src/a"}, "note")))
	chk(t).ok(toolResult(s.findingResolve("F-1", "fixed")))
	chk(t).ok(toolResult(s.findingAdd("trap", []string{"src/a"}, "note")))
}

func TestFindingResolveValidation(t *testing.T) {
	s := server(newBus(t), "a")
	failsWith(t, FindingResolveMissingId)(s.findingResolve(" ", "fixed"))
	failsWith(t, FindingResolveMissingReason)(s.findingResolve("F-1", " "))
	failsWith(t, FindingResolveReasonTooLong)(s.findingResolve("F-1", strings.Repeat("é", 301)))
	failsWith(t, FindingResolveUnknownFinding)(s.findingResolve("F-1", "fixed"))
}

func TestFindingsRequiresRepo(t *testing.T) {
	_, err := newBus(t).findings(" ")
	failsWith(t, FindingsMissingRepo)("", err)
}
