package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestResolveTaskIDMatchesOracle preserves directory precedence and literal substring matching.
func TestResolveTaskIDMatchesOracle(t *testing.T) {
	tasks := t.TempDir()
	for _, name := range []string{"task-alpha", "task-alphabet", "task-beta", ".hidden"} {
		if failure := os.Mkdir(filepath.Join(tasks, name), 0o700); failure != nil {
			t.Fatal(failure)
		}
	}
	writeMetaFixture(t, filepath.Join(tasks, "just-file"), "")
	if failure := os.Symlink("task-beta", filepath.Join(tasks, "linked")); failure != nil {
		t.Fatal(failure)
	}
	if failure := os.Symlink("missing", filepath.Join(tasks, "broken")); failure != nil {
		t.Fatal(failure)
	}
	source, failure := os.ReadFile("../bin/agent")
	if failure != nil {
		t.Fatal(failure)
	}
	start := strings.Index(string(source), "resolve_id() {")
	if start < 0 {
		t.Fatal("oracle resolve_id function not found")
	}
	end := strings.Index(string(source)[start:], "\n}\n")
	if end < 0 {
		t.Fatal("oracle resolve_id function end not found")
	}
	oracle := string(source)[start : start+end+2]
	for _, ref := range []string{"task-alpha", "beta", "alpha", "missing", "just-file", "linked", "broken", ".hidden", "hidden", "", ".", "..", "/", "../" + filepath.Base(tasks) + "/task-beta", "["} {
		got, failure := resolveTaskID(tasks, ref)
		// die's marker captures the exact error text as data without propagating an expected exit.
		want := runMetaBash(t, "C", "TASKS=$1\ndie() { printf 'ERROR:%s' \"$*\"; exit 0; }\n"+oracle+"\nresolve_id \"$2\"", tasks, ref)
		if strings.HasPrefix(want, "ERROR:") {
			if failure == nil || (failure.Cause != ResolveTaskNotFound && failure.Cause != ResolveTaskAmbiguous) {
				t.Fatalf("ref %q: wanted lookup failure, got %q,%v", ref, got, failure)
			}
		} else if failure != nil || got != want {
			t.Fatalf("ref %q: %q,%v want %q", ref, got, failure, want)
		}
	}
	if _, failure := resolveTaskID(tasks, "alpha"); failure == nil || failure.Cause != ResolveTaskAmbiguous {
		t.Fatalf("ambiguous cause = %v", failure)
	}
	if _, failure := resolveTaskID(tasks, "none"); failure == nil || failure.Cause != ResolveTaskNotFound {
		t.Fatalf("missing cause = %v", failure)
	}
	empty := t.TempDir()
	if got, failure := resolveTaskID(empty, "*"); failure != nil || got != "*" {
		t.Fatalf("unexpanded glob quirk lost: %q,%v", got, failure)
	}
	file := filepath.Join(t.TempDir(), "file")
	writeMetaFixture(t, file, "")
	if _, failure := resolveTaskID(file, "x"); failure == nil || failure.Cause != ResolveTaskStorage {
		t.Fatalf("storage cause = %v", failure)
	}
}
