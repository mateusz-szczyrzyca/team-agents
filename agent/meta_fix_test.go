package main

import (
	"errors"
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// TestMissingMetaUpdateFencesRun rejects absent metadata without reviving stale runs.
func TestMissingMetaUpdateFencesRun(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "task"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := setTaskMeta(root, "task", "STATUS", "running", "old-run"); err != nil {
		t.Fatalf("stale run: %v", err)
	}
	if err := setTaskMeta(root, "task", "STATUS", "running", ""); err == nil || err.Cause != SetTaskMetaNotFound {
		t.Fatalf("missing metadata cause: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "task", "meta.env")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("missing metadata was created: %v", err)
	}
}

// TestMetaUpdatePreservesMode keeps metadata readable by its existing consumers.
func TestMetaUpdatePreservesMode(t *testing.T) {
	for _, mode := range []os.FileMode{0644, 0640, 0600} {
		t.Run(mode.String(), func(t *testing.T) {
			root := t.TempDir()
			path := filepath.Join(root, "meta.env")
			if err := os.WriteFile(path, []byte("KEEP=value\n"), 0600); err != nil {
				t.Fatal(err)
			}
			if err := os.Chmod(path, mode); err != nil {
				t.Fatal(err)
			}
			if err := setTaskMeta(root, ".", "STATUS", "running", ""); err != nil {
				t.Fatal(err)
			}
			info, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			if info.Mode().Perm() != mode {
				t.Fatalf("mode = %o, want %o", info.Mode().Perm(), mode)
			}
		})
	}
}

// TestMetaInvalidAssignmentClass preserves the value error and original metadata.
func TestMetaInvalidAssignmentClass(t *testing.T) {
	for _, assignment := range [][2]string{{"BAD-KEY", "value"}, {"KEY", "nul\x00value"}} {
		root := t.TempDir()
		path := filepath.Join(root, "meta.env")
		original := "KEEP=value\n"
		if err := os.WriteFile(path, []byte(original), 0600); err != nil {
			t.Fatal(err)
		}
		err := setTaskMeta(root, ".", assignment[0], assignment[1], "")
		var valueFailure *ValueError
		if err == nil || err.Class() != ClassInvalidArgument || !errors.Is(err, ErrInvalidArgument) || !errors.As(err, &valueFailure) {
			t.Fatalf("assignment error = %v, want InvalidArgument wrapping ValueError", err)
		}
		data, readErr := os.ReadFile(path) // #nosec G304 -- fixture path is owned by t.TempDir.
		if readErr != nil || string(data) != original {
			t.Fatalf("metadata changed: %q, %v", data, readErr)
		}
	}
}

// TestMetaQuotedEmptyJobIDJournals matches Bash's encoded JOB_ID presence check.
func TestMetaQuotedEmptyJobIDJournals(t *testing.T) {
	for _, word := range []string{"''", "\"\"", ""} {
		root := t.TempDir()
		if err := os.WriteFile(filepath.Join(root, "meta.env"), []byte("JOB_ID="+word+"\n"), 0600); err != nil {
			t.Fatal(err)
		}
		if err := setTaskMeta(root, ".", "STATUS", "running", ""); err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(filepath.Join(root, "job_transitions.jsonl")) // #nosec G304 -- fixture path is owned by t.TempDir.
		if word == "" {
			if !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("empty encoded JOB_ID journal: %q, %v", data, err)
			}
		} else if err != nil || len(data) == 0 {
			t.Fatalf("quoted empty JOB_ID %q journal: %q, %v", word, data, err)
		}
	}
}

// TestMetaUnlockFailureRetainsOriginal exposes cleanup errors beside the main cause.
func TestMetaUnlockFailureRetainsOriginal(t *testing.T) {
	f, err := os.CreateTemp(t.TempDir(), "lock")
	if err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	prior := opErr(SetTaskMetaNotFound, "missing metadata")
	result := finishTaskMeta(f, prior)
	if result == nil || result.Cause != SetTaskMetaNotFound || !errors.Is(result, syscall.EBADF) {
		t.Fatalf("cleanup lost original cause or unlock error: %v", result)
	}
	result = finishTaskMeta(f, nil)
	if result == nil || result.Cause != SetTaskMetaStorage || !errors.Is(result, syscall.EBADF) {
		t.Fatalf("cleanup-only failure: %v", result)
	}
}

// TestJobTransitionTimePrecision requires nine fractional digits even at whole seconds.
func TestJobTransitionTimePrecision(t *testing.T) {
	for _, nanos := range []int{0, 100000000, 123456780} {
		at := time.Date(2026, time.November, 5, 16, 0, 0, nanos, time.FixedZone("offset", 3600))
		got := formatJobTransitionTime(at)
		want := at.UTC().Format("2006-01-02T15:04:05.000000000Z")
		if got != want {
			t.Fatalf("timestamp = %q, want %q", got, want)
		}
	}
}
