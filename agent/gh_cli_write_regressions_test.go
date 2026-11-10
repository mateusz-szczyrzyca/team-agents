package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

// TestCLIWriteTransformedBytesContainColors verifies transformed body bytes actually contain color markup.
func TestCLIWriteTransformedBytesContainColors(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args: []string{"pr", "create", "--body", "- agent: `claude-haiku` (model: claude/haiku)"},
		Env: []string{
			"AGENT_REAL_GH=/bin/gh",
			"AGENT_GITHUB_NAME_COLORS=claude-haiku=blue",
		},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	found := false
	for i, arg := range executedCmd.Args {
		if arg == "--body" && i+1 < len(executedCmd.Args) {
			body := executedCmd.Args[i+1]
			if strings.Contains(body, `$\color{blue}{\textbf{claude-haiku}}$`) {
				found = true
			}
		}
	}
	if !found {
		t.Fatalf("body does not contain color markup for agent name: %v", executedCmd.Args)
	}
}

// TestCLIWriteGeneratedLinesRemoved verifies that Generated attribution lines are removed.
func TestCLIWriteGeneratedLinesRemoved(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	body := "Test body\nGenerated with Claude Code\nMore content\n- agent: `test` (model)"
	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body", body},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	found := false
	for i, arg := range executedCmd.Args {
		if arg == "--body" && i+1 < len(executedCmd.Args) {
			transformed := executedCmd.Args[i+1]
			if !strings.Contains(transformed, "Generated with Claude Code") && strings.Contains(transformed, "More content") {
				found = true
			}
		}
	}
	if !found {
		t.Fatalf("Generated line was not removed: %v", executedCmd.Args)
	}
}

// TestCLIWriteTrailingLFPreservedInFile verifies trailing LF is preserved for body-file.
func TestCLIWriteTrailingLFPreservedInFile(t *testing.T) {
	dir := t.TempDir()
	bodyFile := dir + "/body.txt"
	if err := os.WriteFile(bodyFile, []byte("file content\n\n"), 0600); err != nil {
		t.Fatalf("could not create body file: %v", err)
	}

	var readTempContent []byte
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			for i, arg := range cmd.Args {
				if arg == "--body-file" && i+1 < len(cmd.Args) {
					// #nosec G304 -- temp file path from gh_cli_write transformation
					if tmpContent, err := os.ReadFile(cmd.Args[i+1]); err == nil {
						readTempContent = tmpContent
					}
				}
			}
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body-file", bodyFile},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	if len(readTempContent) == 0 {
		t.Fatalf("temp file was not read during execution")
	}
	if !strings.HasSuffix(string(readTempContent), "\n\n") {
		t.Fatalf("transformed file content does not end with double newline: %q", string(readTempContent))
	}
}

// TestCLIWriteFirstBodyFileOnly verifies only first body-file is transformed.
func TestCLIWriteFirstBodyFileOnly(t *testing.T) {
	dir := t.TempDir()
	file1 := dir + "/body1.txt"
	file2 := dir + "/body2.txt"
	if err := os.WriteFile(file1, []byte("first content"), 0600); err != nil {
		t.Fatalf("could not create first body file: %v", err)
	}
	if err := os.WriteFile(file2, []byte("second content"), 0600); err != nil {
		t.Fatalf("could not create second body file: %v", err)
	}

	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body-file", file1, "--body-file", file2},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	tempFileCount := 0
	secondFileUnmodified := false
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			nextArg := executedCmd.Args[i+1]
			if strings.Contains(nextArg, "gh_body_") {
				tempFileCount++
			}
			if nextArg == file2 {
				secondFileUnmodified = true
			}
		}
	}
	if tempFileCount != 1 {
		t.Fatalf("expected 1 temp file, got %d", tempFileCount)
	}
	if !secondFileUnmodified {
		t.Fatalf("second body-file was modified, want %q", file2)
	}
}

// TestCLIWriteFailedReadThenValidFile verifies fallback on failed read with subsequent valid file.
func TestCLIWriteFailedReadThenValidFile(t *testing.T) {
	dir := t.TempDir()
	validFile := dir + "/valid.txt"
	if err := os.WriteFile(validFile, []byte("valid content\n"), 0600); err != nil {
		t.Fatalf("could not create valid file: %v", err)
	}

	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body-file", "/nonexistent/missing.txt", "--body-file", validFile},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	foundMissing := false
	foundTransformedValid := false
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			nextArg := executedCmd.Args[i+1]
			if nextArg == "/nonexistent/missing.txt" {
				foundMissing = true
			}
			if strings.Contains(nextArg, "gh_body_") {
				foundTransformedValid = true
			}
		}
	}
	if !foundMissing {
		t.Fatalf("unreadable file path not forwarded: %v", executedCmd.Args)
	}
	if !foundTransformedValid {
		t.Fatalf("subsequent valid file not transformed: %v", executedCmd.Args)
	}
}

// TestCLIWriteTempCreateFailureStopsBodyTransform verifies that after temp creation fails,
// subsequent --body flags remain untransformed and EventGitHubNamesFailed is logged.
func TestCLIWriteTempCreateFailureStopsBodyTransform(t *testing.T) {
	t.Setenv("TMPDIR", t.TempDir()+"/nonexistent_dir")
	agentsHome := t.TempDir()
	if err := os.Mkdir(filepath.Join(agentsHome, "logs"), 0o700); err != nil {
		t.Fatal(err)
	}

	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	origArgs := []string{"pr", "create", "--body-file", "/any/file.txt", "--body", "- agent: `claude-haiku` (model: claude/haiku)"}
	p := GHDispatchParams{
		Args: append([]string(nil), origArgs...),
		Env: []string{
			"AGENT_REAL_GH=/bin/gh",
			"AGENT_GITHUB_NAME_COLORS=claude-haiku=blue",
			"AGENTS_HOME=" + agentsHome,
		},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	if executedCmd.Name != "/bin/gh" {
		t.Fatalf("expected realGH /bin/gh to be executed, got %q", executedCmd.Name)
	}
	if !slices.Equal(executedCmd.Args, origArgs) {
		t.Fatalf("expected args to remain unchanged %v, got %v", origArgs, executedCmd.Args)
	}

	// Because temp creation failed, loop broke immediately.
	// Neither body-file nor subsequent --body should be transformed.
	for i, arg := range executedCmd.Args {
		if arg == "--body" && i+1 < len(executedCmd.Args) {
			body := executedCmd.Args[i+1]
			if strings.Contains(body, `\color{`) {
				t.Fatalf("subsequent --body was transformed despite temp creation failure: %q", body)
			}
		}
	}

	raw, logErr := fs.ReadFile(os.DirFS(agentsHome), "logs/agent.jsonl")
	if logErr != nil {
		t.Fatalf("could not read log file: %v", logErr)
	}
	var record map[string]string
	if err := json.Unmarshal([]byte(strings.TrimSpace(string(raw))), &record); err != nil {
		t.Fatalf("unmarshal log failed: %v, raw: %s", err, raw)
	}
	if record["event"] != string(EventGitHubNamesFailed) || record["component"] != "agent-gh" || record["level"] != "warn" {
		t.Fatalf("unexpected log record: %v", record)
	}
	if !strings.Contains(record["message"], "no temporary file") {
		t.Fatalf("expected message mentioning no temporary file, got %q", record["message"])
	}
}

// TestCLIWriteCleanupAfterExecutorFailure verifies temp file is removed even when executor fails.
func TestCLIWriteCleanupAfterExecutorFailure(t *testing.T) {
	dir := t.TempDir()
	bodyFile := dir + "/body.txt"
	if err := os.WriteFile(bodyFile, []byte("content"), 0600); err != nil {
		t.Fatalf("could not create body file: %v", err)
	}

	var tempFilePath string
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			for i, arg := range cmd.Args {
				if arg == "--body-file" && i+1 < len(cmd.Args) {
					nextArg := cmd.Args[i+1]
					if strings.Contains(nextArg, "gh_body_") {
						tempFilePath = nextArg
					}
				}
			}
			return CommandResult{ExitCode: 1}, &ExecuteError{Err: errors.New("simulated executor failure")}
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body-file", bodyFile},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if code != 1 || err == nil || err.Cause != GHReadDispatchExecFailed {
		t.Fatalf("expected code 1 and GHReadDispatchExecFailed, got code %d, err %v", code, err)
	}

	if tempFilePath == "" {
		t.Fatalf("temp file was not passed to executor")
	}
	if _, statErr := os.Stat(tempFilePath); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("expected temp file to not exist after executor failure, got: %v", statErr)
	}
}

// TestCLIWriteStdinIdentityPreserved verifies original stdin reader is forwarded to executor.
func TestCLIWriteStdinIdentityPreserved(t *testing.T) {
	originalStdin := strings.NewReader("original stdin")

	var passedStdin io.Reader
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			passedStdin = cmd.Stdin
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body", "test"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  originalStdin,
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	if passedStdin != originalStdin {
		t.Fatalf("stdin identity was not preserved: got %v, want %v", passedStdin, originalStdin)
	}
}

// TestCLIWriteEventGitHubNamesFailedValidation verifies EventGitHubNamesFailed is valid and round-trips JSON.
func TestCLIWriteEventGitHubNamesFailedValidation(t *testing.T) {
	if !EventGitHubNamesFailed.Valid() {
		t.Fatalf("EventGitHubNamesFailed is not valid")
	}
	data, err := json.Marshal(EventGitHubNamesFailed)
	if err != nil {
		t.Fatalf("marshal EventGitHubNamesFailed failed: %v", err)
	}
	if string(data) != `"github_names_failed"` {
		t.Fatalf("marshaled EventGitHubNamesFailed = %s, want %q", data, "github_names_failed")
	}
	var unmarshaled LogEvent
	if err := json.Unmarshal(data, &unmarshaled); err != nil {
		t.Fatalf("unmarshal EventGitHubNamesFailed failed: %v", err)
	}
	if unmarshaled != EventGitHubNamesFailed {
		t.Fatalf("unmarshaled = %s, want %s", unmarshaled, EventGitHubNamesFailed)
	}
}

// TestCLIWriteSourceUnchangedInEnv verifies original environment is not modified.
func TestCLIWriteSourceUnchangedInEnv(t *testing.T) {
	originalEnv := []string{
		"PATH=/usr/bin",
		"AGENT_REAL_GH=/bin/gh",
	}
	envCopy := make([]string, len(originalEnv))
	copy(envCopy, originalEnv)

	fake := fakeGHExec{
		run: func(_ Command) (CommandResult, *ExecuteError) {
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body", "test"},
		Env:    originalEnv,
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	for i, e := range originalEnv {
		if e != envCopy[i] {
			t.Fatalf("environment was modified during handleCLIWrite: %q != %q", e, envCopy[i])
		}
	}
}
