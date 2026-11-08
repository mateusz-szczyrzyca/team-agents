package main

import (
	"bytes"
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
	if err := os.WriteFile(fakeGH, []byte(script), 0o755); err != nil {
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
