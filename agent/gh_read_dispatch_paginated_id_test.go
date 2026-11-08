package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestGHDispatchAPIPaginatedExact64BitIDs asserts exact 64-bit ID preservation and logging across pages.
func TestGHDispatchAPIPaginatedExact64BitIDs(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "logs"), 0o700); err != nil {
		t.Fatal(err)
	}

	pages := []string{
		`[{"id":1234567890123456789,"author":{"login":"alice"},"body":"valid1"},` +
			`{"id":9007199254740993,"author":{"login":"mallory"},"body":"ignored"}]`,
		`[{"id":9876543210987654321,"author":{"login":"alice"},"body":"valid2"}]`,
	}

	var stdout, stderr bytes.Buffer
	exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stdout != nil {
			for _, page := range pages {
				if _, err := cmd.Stdout.Write([]byte(page)); err != nil {
					t.Fatalf("failed writing fake stdout page: %v", err)
				}
			}
		}
		return CommandResult{ExitCode: 0}, nil
	}}

	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"api", "repos/o/r/issues/1/comments", "--paginate"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice", "AGENTS_HOME=" + root},
		Stdout: &stdout,
		Stderr: &stderr,
		Exec:   exec,
	})
	if code != 0 || err != nil {
		t.Fatalf("code: got %d, err: %v", code, err)
	}

	out := stdout.String()
	// Exact retained 64-bit IDs in merged and filtered stdout.
	if !strings.Contains(out, `"id":1234567890123456789`) {
		t.Errorf("stdout missing exact ID 1234567890123456789: %s", out)
	}
	if !strings.Contains(out, `"id":9876543210987654321`) {
		t.Errorf("stdout missing exact ID 9876543210987654321: %s", out)
	}
	// Untrusted comment must be excluded from stdout.
	if strings.Contains(out, "mallory") || strings.Contains(out, "9007199254740993") {
		t.Errorf("stdout should not contain untrusted comment or its ID: %s", out)
	}

	// Verify exact 64-bit ID logging for ignored comment.
	logBytes, logErr := fs.ReadFile(os.DirFS(root), "logs/agent.jsonl")
	if logErr != nil {
		t.Fatalf("failed reading log: %v", logErr)
	}
	logLines := strings.Split(strings.TrimSpace(string(logBytes)), "\n")
	foundLoggedID := false
	for _, line := range logLines {
		var record map[string]string
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Fatalf("malformed log record %q: %v", line, err)
		}
		if record["event"] == string(EventGitHubCommentIgnored) && record["author"] == "mallory" {
			if record["id"] != "9007199254740993" {
				t.Errorf("logged id: got %q, want exact 64-bit string %q", record["id"], "9007199254740993")
			}
			foundLoggedID = true
		}
	}
	if !foundLoggedID {
		t.Errorf("missing logged warning for untrusted comment: %s", string(logBytes))
	}

	// Additional check: trusted 64-bit ID 9007199254740993 is retained exactly without float64 precision loss.
	var stdout2, stderr2 bytes.Buffer
	trustedPages := []string{
		`[{"id":9007199254740993,"author":{"login":"alice"},"body":"valid3"}]`,
		`[{"id":9876543210987654321,"author":{"login":"alice"},"body":"valid4"}]`,
	}
	exec2 := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stdout != nil {
			for _, page := range trustedPages {
				if _, err := cmd.Stdout.Write([]byte(page)); err != nil {
					t.Fatalf("failed writing fake stdout page: %v", err)
				}
			}
		}
		return CommandResult{ExitCode: 0}, nil
	}}
	code2, err2 := ghDispatch(context.Background(), GHDispatchParams{
		Args:   []string{"api", "repos/o/r/issues/1/comments", "--paginate"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice", "AGENTS_HOME=" + root},
		Stdout: &stdout2,
		Stderr: &stderr2,
		Exec:   exec2,
	})
	if code2 != 0 || err2 != nil {
		t.Fatalf("code2: got %d, err2: %v", code2, err2)
	}
	out2 := stdout2.String()
	if !strings.Contains(out2, `"id":9007199254740993`) {
		t.Errorf("stdout2 missing exact ID 9007199254740993 (might have been corrupted by float64): %s", out2)
	}
	if strings.Contains(out2, "9007199254740992") {
		t.Errorf("stdout2 has corrupted float64 precision loss 9007199254740992: %s", out2)
	}
}
