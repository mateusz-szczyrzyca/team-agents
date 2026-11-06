package main

import (
	"bytes"
	"errors"
	"io"
	"os"
	"path/filepath"
	"testing"
)

// writeAckFixture creates inert metadata in an isolated task root.
func writeAckFixture(t *testing.T, root, id, status string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(root, id), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, id, "meta.env"), []byte("ID="+id+"\nSTATUS="+status+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
}

// Rule: only the five Bash terminal statuses can be acknowledged individually.
func TestAckTerminalStatuses(t *testing.T) {
	for _, status := range []string{"done", "failed", "stopped", "excluded", "hung", "running", "starting", "waiting", "sleeping", "paused", "new"} {
		t.Run(status, func(t *testing.T) {
			root := t.TempDir()
			writeAckFixture(t, root, "task-one", status)
			var out bytes.Buffer // out captures the exact command contract.
			err := (&App{}).cmdAck("task-one", &out, root)
			terminal := status == "done" || status == "failed" || status == "stopped" || status == "excluded" || status == "hung" // terminal follows the Bash allowlist.
			if terminal {
				if err != nil || out.String() != "Acknowledged task-one.\n" {
					t.Fatalf("output=%q error=%v", out.String(), err)
				}
				if _, statErr := os.Stat(filepath.Join(root, "task-one", "acked")); statErr != nil {
					t.Fatal(statErr)
				}
				return
			}
			if err == nil || err.Cause != AckStillRunning {
				t.Fatalf("want AckStillRunning, got %v", err)
			}
			if _, statErr := os.Stat(filepath.Join(root, "task-one", "acked")); !errors.Is(statErr, os.ErrNotExist) {
				t.Fatalf("nonterminal task has acknowledgement: %v", statErr)
			}
		})
	}
}

// Rule: all counts terminal tasks on every call and leaves other statuses untouched.
func TestAckAll(t *testing.T) {
	root := t.TempDir()
	for _, status := range []string{"done", "failed", "stopped", "excluded", "hung", "running", "waiting"} {
		writeAckFixture(t, root, status, status)
	}
	for range 2 {
		var out bytes.Buffer // out checks acknowledgement counts, including repeated calls.
		if err := (&App{}).cmdAck("all", &out, root); err != nil || out.String() != "Acknowledged: 5\n" {
			t.Fatalf("output=%q error=%v", out.String(), err)
		}
	}
	for _, status := range []string{"done", "failed", "stopped", "excluded", "hung", "running", "waiting"} {
		_, err := os.Stat(filepath.Join(root, status, "acked"))
		if (status == "running" || status == "waiting") != errors.Is(err, os.ErrNotExist) {
			t.Fatalf("status=%s marker error=%v", status, err)
		}
	}
}

// Rule: ID resolution preserves unique fragments, missing and ambiguous causes.
func TestAckResolve(t *testing.T) {
	root := t.TempDir()
	writeAckFixture(t, root, "one-alpha", "done")
	writeAckFixture(t, root, "two-alpha", "done")
	if err := (&App{}).cmdAck("missing", io.Discard, root); err == nil || err.Cause != AckNotFound {
		t.Fatalf("want AckNotFound, got %v", err)
	}
	if err := (&App{}).cmdAck("alpha", io.Discard, root); err == nil || err.Cause != AckAmbiguous {
		t.Fatalf("want AckAmbiguous, got %v", err)
	}
	var out bytes.Buffer // out verifies the resolved full ID.
	if err := (&App{}).cmdAck("one-", &out, root); err != nil || out.String() != "Acknowledged one-alpha.\n" {
		t.Fatalf("output=%q error=%v", out.String(), err)
	}
}

// Rule: acknowledging again truncates the marker, matching Bash redirection.
func TestAckTruncatesExistingMarker(t *testing.T) {
	root := t.TempDir()
	writeAckFixture(t, root, "task", "done")
	marker := filepath.Join(root, "task", "acked") // marker may already carry external annotation.
	if err := os.WriteFile(marker, []byte("keep"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := (&App{}).cmdAck("task", io.Discard, root); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(marker)
	if err != nil {
		t.Fatal(err)
	}
	if info.Size() != 0 {
		t.Fatalf("marker size=%d, want zero", info.Size())
	}
}

// ackFailWriter exercises stdout errors without any external process.
type ackFailWriter struct{}

// Write returns a sentinel for assertions through the typed storage wrapper.
func (ackFailWriter) Write([]byte) (int, error) { return 0, io.ErrClosedPipe }

// Rule: errors writing command output remain observable as AckStorage.
func TestAckOutputError(t *testing.T) {
	root := t.TempDir()
	writeAckFixture(t, root, "task", "done")
	err := (&App{}).cmdAck("task", ackFailWriter{}, root)
	if err == nil || err.Cause != AckStorage || !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("want AckStorage wrapping closed pipe, got %v", err)
	}
}

// Rule: a marker write failure returns storage failure in both target modes.
func TestAckMarkerFailure(t *testing.T) {
	for _, target := range []string{"task", "all"} {
		t.Run(target, func(t *testing.T) {
			root := t.TempDir()
			writeAckFixture(t, root, "task", "done")
			if err := os.Mkdir(filepath.Join(root, "task", "acked"), 0o700); err != nil {
				t.Fatal(err)
			}
			err := (&App{}).cmdAck(target, io.Discard, root)
			if err == nil || err.Cause != AckStorage {
				t.Fatalf("want AckStorage, got %v", err)
			}
		})
	}
}

// Rule: all ignores hidden directories and entries without a regular metadata file.
func TestAckAllIgnoresNonTaskDirectories(t *testing.T) {
	root := t.TempDir()
	writeAckFixture(t, root, "visible", "done")
	writeAckFixture(t, root, ".hidden", "done")
	if err := os.MkdirAll(filepath.Join(root, "directory-meta", "meta.env"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(root, "no-meta"), 0o700); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer // out verifies only the real, visible task is counted.
	if err := (&App{}).cmdAck("all", &out, root); err != nil || out.String() != "Acknowledged: 1\n" {
		t.Fatalf("output=%q error=%v", out.String(), err)
	}
}
