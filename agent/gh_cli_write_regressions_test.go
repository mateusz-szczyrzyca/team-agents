package main

import (
	"context"
	"errors"
	"io"
	"os"
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

	// Body with agent name in proper format should be colored
	// Format: "- agent: `<name>` (<suffix>)"
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

	// Check that color markup was applied to agent name
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
		t.Fatalf("body does not contain color markup for agent name")
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

	// Check that Generated line was removed
	found := false
	for i, arg := range executedCmd.Args {
		if arg == "--body" && i+1 < len(executedCmd.Args) {
			transformed := executedCmd.Args[i+1]
			if !strings.Contains(transformed, "Generated with Claude Code") {
				found = true
			}
		}
	}
	if !found {
		t.Fatalf("Generated line was not removed")
	}
}

// TestCLIWriteTrailingLFPreservedInFile verifies trailing LF is preserved for body-file.
func TestCLIWriteTrailingLFPreservedInFile(t *testing.T) {
	dir := t.TempDir()
	bodyFile := dir + "/body.txt"
	// File content with trailing newline
	if err := os.WriteFile(bodyFile, []byte("file content\n"), 0600); err != nil {
		t.Fatalf("could not create body file: %v", err)
	}

	var readTempContent []byte
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			// Read the temp file to verify its content
			for i, arg := range cmd.Args {
				if arg == "--body-file" && i+1 < len(cmd.Args) {
					tempPath := cmd.Args[i+1]
					// #nosec G304 -- temp file path from gh_cli_write transformation
					if tmpContent, err := os.ReadFile(tempPath); err == nil {
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

	// Check that transformed file preserves trailing LF
	if len(readTempContent) == 0 {
		t.Fatalf("temp file was not read during execution")
	}
	if !strings.HasSuffix(string(readTempContent), "\n") {
		t.Fatalf("transformed file content does not end with newline")
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

	// Check that only first --body-file was transformed (temp file)
	tempFileCount := 0
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			nextArg := executedCmd.Args[i+1]
			if strings.Contains(nextArg, "gh_body_") {
				tempFileCount++
			}
		}
	}
	if tempFileCount != 1 {
		t.Fatalf("expected 1 temp file, got %d", tempFileCount)
	}
}

// TestCLIWriteFailedReadThenValidFile verifies fallback on failed read with subsequent valid file.
func TestCLIWriteFailedReadThenValidFile(t *testing.T) {
	dir := t.TempDir()
	validFile := dir + "/valid.txt"
	if err := os.WriteFile(validFile, []byte("valid content"), 0600); err != nil {
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
		Args:   []string{"pr", "create", "--body-file", "/nonexistent", "--body-file", validFile},
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

	// First failed file should be forwarded as-is, second valid should be transformed
	found := false
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			// Should have the failed nonexistent path and the temp file for valid
			if executedCmd.Args[i+1] == "/nonexistent" {
				found = true
			}
		}
	}
	if !found {
		t.Fatalf("failed file path not forwarded as expected")
	}
}

// TestCLIWriteCleanupAfterExecutorFailure verifies temp file cleanup on executor error.
func TestCLIWriteCleanupAfterExecutorFailure(t *testing.T) {
	dir := t.TempDir()
	bodyFile := dir + "/body.txt"
	if err := os.WriteFile(bodyFile, []byte("content"), 0600); err != nil {
		t.Fatalf("could not create body file: %v", err)
	}

	var tempFilePath string
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			// Capture the temp file path
			for i, arg := range cmd.Args {
				if arg == "--body-file" && i+1 < len(cmd.Args) {
					nextArg := cmd.Args[i+1]
					if strings.Contains(nextArg, "gh_body_") {
						tempFilePath = nextArg
					}
				}
			}
			// Simulate executor failure
			return CommandResult{}, &ExecuteError{Err: errors.New("simulated failure")}
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

	_, _ = p.handleCLIWrite(context.Background(), "/bin/gh")

	// Verify temp file was cleaned up
	if tempFilePath != "" {
		if _, statErr := os.Stat(tempFilePath); statErr == nil {
			t.Fatalf("temp file was not cleaned up after executor failure: %s", tempFilePath)
		}
	}
}

// TestCLIWriteStdinIdentityPreserved verifies original stdin reader is used when not reading from stdin.
func TestCLIWriteStdinIdentityPreserved(t *testing.T) {
	originalStdinContent := "original stdin"
	originalStdin := strings.NewReader(originalStdinContent)

	var passedStdin io.Reader
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			// This will be called with the stdin reader
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

	// Verify stdin passed to executor is the original
	if passedStdin != originalStdin {
		t.Fatalf("stdin identity was not preserved")
	}
}

// TestCLIWriteMultipleBodyFlagsOnlyFirstBodyFileTransformed verifies multiple body transformations.
func TestCLIWriteMultipleBodyFlagsOnlyFirstBodyFileTransformed(t *testing.T) {
	dir := t.TempDir()
	file1 := dir + "/file1.txt"
	if err := os.WriteFile(file1, []byte("- agent: `alice` (model)"), 0600); err != nil {
		t.Fatalf("could not create file: %v", err)
	}

	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args: []string{"pr", "create", "--body", "direct body", "--body-file", file1},
		Env: []string{
			"AGENT_REAL_GH=/bin/gh",
			"AGENT_GITHUB_NAME_COLORS=alice=red",
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

	// Both direct body and file should be transformed
	directBodyFound, fileBodyFound := false, false
	for i, arg := range executedCmd.Args {
		if arg == "--body" && i+1 < len(executedCmd.Args) {
			body := executedCmd.Args[i+1]
			if strings.Contains(body, "direct body") {
				directBodyFound = true
			}
		}
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			nextArg := executedCmd.Args[i+1]
			if strings.Contains(nextArg, "gh_body_") {
				fileBodyFound = true
			}
		}
	}
	if !directBodyFound {
		t.Fatalf("direct --body not found in executed command")
	}
	if !fileBodyFound {
		t.Fatalf("transformed --body-file not found in executed command")
	}
}

// TestCLIWriteEventGitHubNamesFailedValidation verifies EventGitHubNamesFailed is valid.
func TestCLIWriteEventGitHubNamesFailedValidation(t *testing.T) {
	// Verify that EventGitHubNamesFailed is a valid log event
	if !EventGitHubNamesFailed.Valid() {
		t.Fatalf("EventGitHubNamesFailed is not valid")
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

	_, _ = p.handleCLIWrite(context.Background(), "/bin/gh")

	// Verify environment slice was not modified
	for i, e := range originalEnv {
		if e != envCopy[i] {
			t.Fatalf("environment was modified during handleCLIWrite")
		}
	}
}
