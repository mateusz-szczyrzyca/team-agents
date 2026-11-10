package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestCanonicalGitHubRepoURL_Parity verifies exact Bash prefix, whitespace, and length boundaries.
func TestCanonicalGitHubRepoURL_Parity(t *testing.T) {
	long100 := strings.Repeat("a", 100)
	long101 := strings.Repeat("a", 101)

	tests := []struct {
		name  string
		input string
		want  string
	}{
		{"git@ with slash rejected", "git@github.com/owner/repo", ""},
		{"ssh with colon rejected", "ssh://git@github.com:owner/repo", ""},
		{"leading whitespace rejected", " https://github.com/owner/repo", ""},
		{"trailing space rejected", "https://github.com/owner/repo ", ""},
		{"repo exactly 100 chars allowed", "https://github.com/owner/" + long100, "https://github.com/owner/" + long100},
		{"repo 101 chars rejected", "https://github.com/owner/" + long101, ""},
		{"nested path components rejected", "https://github.com/owner/repo/extra", ""},
		{"valid git@ with colon", "git@github.com:owner/repo", "https://github.com/owner/repo"},
		{"valid ssh with slash", "ssh://git@github.com/owner/repo", "https://github.com/owner/repo"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := CanonicalGitHubRepoURL(tc.input)
			if got != tc.want {
				t.Fatalf("CanonicalGitHubRepoURL(%q) = %q, want %q", tc.input, got, tc.want)
			}
		})
	}
}

// TestResolveGitHubRepoURL_LoggingAndTrailingLF verifies warning logging on git errors and trailing LF handling.
func TestResolveGitHubRepoURL_LoggingAndTrailingLF(t *testing.T) {
	ctx := context.Background()

	t.Run("git exit 2 silent", func(t *testing.T) {
		tempDir := t.TempDir()
		var stderr bytes.Buffer
		logger := &Logger{LogDir: tempDir, Stderr: &stderr}
		exec := fakeGHExec{
			run: func(_ Command) (CommandResult, *ExecuteError) {
				return CommandResult{ExitCode: 2}, nil
			},
		}
		got := ResolveGitHubRepoURL(ctx, exec, "/repo", logger)
		if got != "" {
			t.Fatalf("got %q, want empty", got)
		}
		if stderr.Len() != 0 {
			t.Fatalf("expected silent exit 2, got stderr: %s", stderr.String())
		}
		if _, err := os.Stat(filepath.Join(tempDir, "agent.jsonl")); !os.IsNotExist(err) {
			t.Fatalf("expected no agent.jsonl written for exit 2, err: %v", err)
		}
	})

	t.Run("git exit 128 logs warning", func(t *testing.T) {
		tempDir := t.TempDir()
		var stderr bytes.Buffer
		logger := &Logger{LogDir: tempDir, Stderr: &stderr}
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil {
					_, _ = cmd.Stdout.Write([]byte("fatal: not a git repository\n"))
				}
				return CommandResult{ExitCode: 128}, nil
			},
		}
		got := ResolveGitHubRepoURL(ctx, exec, "/test/repo", logger)
		if got != "" {
			t.Fatalf("got %q, want empty", got)
		}
		logBytes, err := os.ReadFile(filepath.Clean(filepath.Join(tempDir, "agent.jsonl"))) //nolint:gosec // G304: test-controlled path in temp directory
		if err != nil {
			t.Fatalf("failed to read agent.jsonl: %v", err)
		}
		out := string(logBytes)
		if !strings.Contains(out, string(LogJobOriginLookupFailed)) {
			t.Fatalf("expected log %q in agent.jsonl, got: %s", LogJobOriginLookupFailed, out)
		}
		if !strings.Contains(out, `"repository":"/test/repo"`) || !strings.Contains(out, `"exit":"128"`) {
			t.Fatalf("expected fields in agent.jsonl, got: %s", out)
		}
	})

	t.Run("trailing newline stripped like shell", func(t *testing.T) {
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil {
					_, _ = cmd.Stdout.Write([]byte("https://github.com/owner/repo\n\n"))
				}
				return CommandResult{ExitCode: 0}, nil
			},
		}
		got := ResolveGitHubRepoURL(ctx, exec, "/repo", nil)
		if got != "https://github.com/owner/repo" {
			t.Fatalf("got %q, want %q", got, "https://github.com/owner/repo")
		}
	})
}

// TestCanonicalGitHubRepoURL_BashParity verifies exact parity with the frozen Bash function.
func TestCanonicalGitHubRepoURL_BashParity(t *testing.T) {
	script := `
github_repo_url() {
  local url tail owner repo rc=0
  url=$1
  case $url in
    https://github.com/*) tail=${url#https://github.com/} ;;
    git@github.com:*) tail=${url#git@github.com:} ;;
    ssh://git@github.com/*) tail=${url#ssh://git@github.com/} ;;
    *) return 0 ;;
  esac
  tail=${tail%.git}; owner=${tail%%/*}; repo=${tail#*/}
  if [[ $tail == */* && $owner =~ ^[A-Za-z0-9][A-Za-z0-9-]{0,38}$ && $repo =~ ^[A-Za-z0-9_.-]{1,100}$ && $repo != . && $repo != .. ]]; then
    printf 'https://github.com/%s/%s' "$owner" "$repo"
  fi
}
github_repo_url "$1"
`
	long100 := strings.Repeat("x", 100)
	long101 := strings.Repeat("x", 101)

	inputs := []string{
		"https://github.com/owner/repo",
		"https://github.com/owner/repo.git",
		"git@github.com:owner/repo",
		"git@github.com:owner/repo.git",
		"git@github.com/owner/repo",
		"ssh://git@github.com/owner/repo",
		"ssh://git@github.com:owner/repo",
		"https://github.com/owner/" + long100,
		"https://github.com/owner/" + long101,
		" https://github.com/owner/repo",
		"https://github.com/owner/repo ",
		"https://github.com/owner/.",
		"https://github.com/owner/..",
		"https://github.com/-invalid/repo",
	}

	for _, input := range inputs {
		t.Run(filepath.Base(input), func(t *testing.T) {
			oracle := runMetaBash(t, "C.UTF-8", script, input)
			got := CanonicalGitHubRepoURL(input)
			if got != oracle {
				t.Fatalf("CanonicalGitHubRepoURL(%q) = %q, oracle = %q", input, got, oracle)
			}
		})
	}
}
