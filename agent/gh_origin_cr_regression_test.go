package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"unicode/utf8"
)

// TestResolveGitHubRepoURL_CRLFAndCRRejection verifies that CR is not stripped like LF.
func TestResolveGitHubRepoURL_CRLFAndCRRejection(t *testing.T) {
	ctx := context.Background()
	for _, input := range []string{"https://github.com/owner/repo\r\n", "https://github.com/owner/repo\r"} {
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Stdout != nil {
				if _, err := cmd.Stdout.Write([]byte(input)); err != nil {
					t.Fatalf("failed to write stdout: %v", err)
				}
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		if got := ResolveGitHubRepoURL(ctx, exec, "/repo", nil); got != "" {
			t.Fatalf("ResolveGitHubRepoURL(%q) = %q, want empty", input, got)
		}
	}
}

// TestResolveGitHubRepoURL_ErrorHandling verifies process start failure, logging failure, and rune truncation.
func TestResolveGitHubRepoURL_ErrorHandling(t *testing.T) {
	ctx := context.Background()
	tempDir := t.TempDir()
	logger := &Logger{LogDir: tempDir}

	execErr := fakeGHExec{run: func(_ Command) (CommandResult, *ExecuteError) {
		return CommandResult{}, opErr(ExecuteStart, "git exec error")
	}}
	if got := ResolveGitHubRepoURL(ctx, execErr, "/repo", logger); got != "" {
		t.Fatalf("expected empty for start failure, got %q", got)
	}

	execRune := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stdout != nil {
			if _, err := cmd.Stdout.Write([]byte(strings.Repeat("日", 305) + "\n")); err != nil {
				t.Fatalf("failed to write stdout: %v", err)
			}
		}
		return CommandResult{ExitCode: 1}, nil
	}}
	_ = ResolveGitHubRepoURL(ctx, execRune, "/repo", logger)
	_ = ResolveGitHubRepoURL(ctx, execRune, "/repo", &Logger{LogDir: "/dev/null/forbidden"})

	logBytes, err := os.ReadFile(filepath.Clean(filepath.Join(tempDir, "agent.jsonl"))) //nolint:gosec // G304: test path in temp dir
	if err != nil {
		t.Fatalf("failed to read agent.jsonl: %v", err)
	}
	lines := strings.Split(strings.TrimSpace(string(logBytes)), "\n")
	if len(lines) < 2 || !strings.Contains(lines[0], "git exec error") {
		t.Fatalf("expected start error in first line, got: %s", logBytes)
	}
	var rec map[string]any
	if err := json.Unmarshal([]byte(lines[1]), &rec); err != nil {
		t.Fatalf("failed to parse log line: %v", err)
	}
	if reason, _ := rec["reason"].(string); utf8.RuneCountInString(reason) != 300 {
		t.Fatalf("expected 300 runes in reason, got %d", utf8.RuneCountInString(reason))
	}
}
