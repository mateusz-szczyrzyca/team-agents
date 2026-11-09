package main

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestRunGHReportsInvalidJSONDiagnostic verifies that malformed JSON responses
// from real gh emit an agent-gh diagnostic on stderr while returning exit code 1.
func TestRunGHReportsInvalidJSONDiagnostic(t *testing.T) {
	root := t.TempDir()
	fakeGH := filepath.Join(root, "real-gh")
	script := "#!/bin/sh\nprintf 'not-valid-json' >&1\nexit 0\n"
	if err := os.WriteFile(fakeGH, []byte(script), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(fakeGH, 0o700); err != nil { // #nosec G302 -- isolated test fixture must be executable
		t.Fatal(err)
	}

	var stdout, stderr bytes.Buffer
	env := []string{
		"AGENT_REAL_GH=" + fakeGH,
		"AGENT_GITHUB_TRUSTED_RESOLVED=alice",
	}
	code := runGH([]string{"api", "repos/o/r/issues/1/comments"}, env, &stdout, &stderr)
	if code != 1 {
		t.Fatalf("exit code = %d, want 1", code)
	}
	if !strings.Contains(stderr.String(), "agent-gh:") {
		t.Fatalf("stderr does not contain diagnostic: %q", stderr.String())
	}
}

// TestRunGHDiagnosticWriterFailurePreservesCode verifies that when writing the diagnostic to stderr
// fails, runGH returns exit code 1 or the non-zero status instead of failing silently.
func TestRunGHDiagnosticWriterFailurePreservesCode(t *testing.T) {
	root := t.TempDir()
	fakeGH := filepath.Join(root, "real-gh")
	script := "#!/bin/sh\nprintf 'not-valid-json' >&1\nexit 0\n"
	if err := os.WriteFile(fakeGH, []byte(script), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(fakeGH, 0o700); err != nil { // #nosec G302 -- isolated test fixture must be executable
		t.Fatal(err)
	}

	var stdout bytes.Buffer
	env := []string{
		"AGENT_REAL_GH=" + fakeGH,
		"AGENT_GITHUB_TRUSTED_RESOLVED=alice",
	}
	code := runGH([]string{"api", "repos/o/r/issues/1/comments"}, env, &stdout, failWriter{err: io.ErrShortWrite})
	if code != 1 {
		t.Fatalf("exit code = %d, want 1", code)
	}
}

// TestRunGHDiagnosticWriterFailurePreservesCode127 verifies that when diagnostic writing
// to stderr fails during a storage error (e.g. MissingRealGH refusal with failing stderr),
// runGH preserves the specific exit code (127) rather than defaulting to 1.
func TestRunGHDiagnosticWriterFailurePreservesCode127(t *testing.T) {
	root := t.TempDir()
	nonexistentGH := filepath.Join(root, "nonexistent-gh")
	var stdout bytes.Buffer
	env := []string{
		"AGENT_REAL_GH=" + nonexistentGH,
	}
	code := runGH([]string{"api", "repos/o/r/issues/1/comments"}, env, &stdout, failWriter{err: io.ErrShortWrite})
	if code != 127 {
		t.Fatalf("exit code = %d, want 127", code)
	}
}

// TestRunGHSpawnFailureReportsDiagnostic verifies that when real gh cannot be spawned
// (e.g. invalid executable format / ENOEXEC), a typed ExecFailed diagnostic is reported on stderr.
func TestRunGHSpawnFailureReportsDiagnostic(t *testing.T) {
	root := t.TempDir()
	badExec := filepath.Join(root, "bad-exec")
	if err := os.WriteFile(badExec, []byte("garbage binary format"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(badExec, 0o700); err != nil { // #nosec G302 -- isolated test fixture must be executable
		t.Fatal(err)
	}

	var stdout, stderr bytes.Buffer
	env := []string{
		"AGENT_REAL_GH=" + badExec,
		"AGENT_GITHUB_TRUSTED_RESOLVED=alice",
	}
	code := runGH([]string{"api", "repos/o/r/issues/1/comments"}, env, &stdout, &stderr)
	if code == 0 {
		t.Fatalf("exit code = %d, want non-zero", code)
	}
	if !strings.Contains(stderr.String(), "agent-gh:") {
		t.Fatalf("stderr does not contain diagnostic: %q", stderr.String())
	}
}
