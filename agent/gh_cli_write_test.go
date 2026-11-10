package main

import (
	"context"
	"io"
	"os"
	"strings"
	"testing"
)

// TestHandleCLIWriteBodyFlag tests --body flag transformation.
func TestHandleCLIWriteBodyFlag(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body", "test body"},
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// Body should be transformed (LF stripped after transformation)
	foundBody := false
	for i, arg := range executedCmd.Args {
		if arg == "--body" && i+1 < len(executedCmd.Args) {
			foundBody = true
			body := executedCmd.Args[i+1]
			if body != "test body" {
				t.Fatalf("body not transformed correctly: got %q, want %q", body, "test body")
			}
		}
	}
	if !foundBody {
		t.Fatalf("--body flag not found in args: %v", executedCmd.Args)
	}
}

// TestHandleCLIWriteBodyEqualsFormat tests --body=value format.
func TestHandleCLIWriteBodyEqualsFormat(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "edit", "--body=test value"},
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// Check body= format
	foundBody := false
	for _, arg := range executedCmd.Args {
		if strings.HasPrefix(arg, "--body=") {
			foundBody = true
			expected := "--body=test value"
			if arg != expected {
				t.Fatalf("body= not transformed correctly: got %q, want %q", arg, expected)
			}
		}
	}
	if !foundBody {
		t.Fatalf("--body= flag not found in args: %v", executedCmd.Args)
	}
}

// TestHandleCLIWritePassthrough tests commands without body flags pass through unchanged.
func TestHandleCLIWritePassthrough(t *testing.T) {
	executed := false
	fake := fakeGHExec{
		run: func(_ Command) (CommandResult, *ExecuteError) {
			executed = true
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "view", "123"},
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}
	if !executed {
		t.Fatalf("command was not executed")
	}
}

// TestHandleCLIWriteBodyFile tests --body-file with file transformation.
func TestHandleCLIWriteBodyFile(t *testing.T) {
	dir := t.TempDir()
	bodyFile := dir + "/body.txt"
	if err := os.WriteFile(bodyFile, []byte("file content"), 0600); err != nil {
		t.Fatalf("could not create body file: %v", err)
	}

	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// Check that a temp file was created and used
	foundTempFile := false
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			nextArg := executedCmd.Args[i+1]
			if nextArg != bodyFile && strings.Contains(nextArg, "gh_body_") {
				foundTempFile = true
				// Temp file should be cleaned up after execution
				if _, statErr := os.Stat(nextArg); statErr == nil {
					t.Fatalf("temp file was not cleaned up: %s", nextArg)
				}
			}
		}
	}
	if !foundTempFile {
		t.Fatalf("temp file not created for body-file: %v", executedCmd.Args)
	}
}

// TestHandleCLIWriteUnreadableFile tests --body-file with unreadable file fallback.
func TestHandleCLIWriteUnreadableFile(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body-file", "/nonexistent/file.txt"},
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// Original unreadable file path should be forwarded unchanged
	foundUnreadable := false
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) && executedCmd.Args[i+1] == "/nonexistent/file.txt" {
			foundUnreadable = true
		}
	}
	if !foundUnreadable {
		t.Fatalf("unreadable file path not forwarded: %v", executedCmd.Args)
	}
}

// TestHandleCLIWriteStdinBody tests --body-file with stdin input.
func TestHandleCLIWriteStdinBody(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	stdinContent := "stdin body content"
	p := GHDispatchParams{
		Args:   []string{"pr", "create", "--body-file", "-"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh"},
		Stdin:  strings.NewReader(stdinContent),
		Stdout: io.Discard,
		Stderr: io.Discard,
		Exec:   fake,
	}

	code, err := p.handleCLIWrite(context.Background(), "/bin/gh")
	if err != nil {
		t.Fatalf("handleCLIWrite failed: %v", err)
	}
	if code != 0 {
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// stdin should be consumed, so temp file should be used
	foundTempFile := false
	for i, arg := range executedCmd.Args {
		if arg == "--body-file" && i+1 < len(executedCmd.Args) {
			if strings.Contains(executedCmd.Args[i+1], "gh_body_") {
				foundTempFile = true
			}
		}
	}
	if !foundTempFile {
		t.Fatalf("stdin body-file not processed: %v", executedCmd.Args)
	}
}

// TestHandleCLIWriteNotApplicableVerb tests that non-pr/issue commands pass through unchanged.
func TestHandleCLIWriteNotApplicableVerb(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"repo", "create", "--description", "test"},
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// Args should be unchanged
	if len(executedCmd.Args) != len(p.Args) {
		t.Fatalf("args were modified for non-applicable verb: %v", executedCmd.Args)
	}
}

// TestHandleCLIWriteNotApplicableSubcommand tests that only create/edit/comment are transformed.
func TestHandleCLIWriteNotApplicableSubcommand(t *testing.T) {
	var executedCmd Command
	fake := fakeGHExec{
		run: func(cmd Command) (CommandResult, *ExecuteError) {
			executedCmd = cmd
			return CommandResult{ExitCode: 0}, nil
		},
	}

	p := GHDispatchParams{
		Args:   []string{"pr", "view", "--body", "should not transform"},
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
		t.Fatalf("handleCLIWrite returned code %d, want 0", code)
	}

	// Args should be unchanged
	if len(executedCmd.Args) != len(p.Args) {
		t.Fatalf("args were modified for non-applicable subcommand: %v", executedCmd.Args)
	}
}
