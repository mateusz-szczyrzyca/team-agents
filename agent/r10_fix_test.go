package main

import (
	"bytes"
	"context"
	"io"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

// writeTestScript creates a file or script in a test directory.
func writeTestScript(t *testing.T, path, content string, mode os.FileMode) {
	if err := os.MkdirAll(filepath.Dir(path), 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), mode); err != nil {
		t.Fatal(err)
	}
}

// TestEffectiveChildPathResolution verifies that bare commands resolve from child PATH.
func TestEffectiveChildPathResolution(t *testing.T) {
	dirA, dirB := t.TempDir(), t.TempDir()
	writeTestScript(t, filepath.Join(dirA, "samplecmd"), "#!/bin/sh\necho parent\n", 0o755)
	writeTestScript(t, filepath.Join(dirB, "samplecmd"), "#!/bin/sh\necho child\n", 0o755)
	var out bytes.Buffer
	res, err := OSExecutor{}.Execute(context.Background(), Command{Name: "samplecmd", Env: []string{"PATH=" + dirB}, Stdout: &out})
	if err != nil || res.ExitCode != 0 || strings.TrimSpace(out.String()) != "child" {
		t.Fatalf("err=%v exit=%d out=%q", err, res.ExitCode, out.String())
	}
}

// TestDelegatedBusPartialStdoutAndRawExit verifies partial stdout retention and raw exit preservation.
func TestDelegatedBusPartialStdoutAndRawExit(t *testing.T) {
	app := &App{Executor: executorFunc(func(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stdout != nil {
			_, _ = cmd.Stdout.Write([]byte("partial-text\n"))
		}
		return CommandResult{ExitCode: 42}, nil
	})}
	var out bytes.Buffer
	if code := app.run([]string{"helpers"}, &out, io.Discard); code != 42 || out.String() != "partial-text\n" {
		t.Fatalf("got exit=%d out=%q, want exit=42 out=partial-text\\n", code, out.String())
	}
}

// TestCmdShowOptionalFilesDirectoriesAndCRLF verifies that directories for optional files do not fail show.
func TestCmdShowOptionalFilesDirectoriesAndCRLF(t *testing.T) {
	tasksDir := t.TempDir()
	taskDir := filepath.Join(tasksDir, "task-opt")
	for _, sub := range []string{"final.md", "result.md"} {
		if err := os.MkdirAll(filepath.Join(taskDir, sub), 0o750); err != nil {
			t.Fatal(err)
		}
	}
	writeTestScript(t, filepath.Join(taskDir, "meta.env"), "ID=task-opt\nKIND=investigation\n", 0o644)
	writeTestScript(t, filepath.Join(taskDir, "task.md"), "Task text\n", 0o644)
	app := &App{tasksDir: tasksDir}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "task-opt", &out); err != nil {
		t.Fatalf("cmdShow failed on directory optional files: %v", err)
	}
}

// TestStatusAndAgentsSystemctlFailure verifies that a failing systemctl query is treated as inactive.
func TestStatusAndAgentsSystemctlFailure(t *testing.T) {
	tasksDir := t.TempDir()
	taskDir := filepath.Join(tasksDir, "task-sys")
	writeTestScript(t, filepath.Join(taskDir, "meta.env"), "ID=task-sys\nSTATUS=running\nAGENT=solo\n", 0o644)
	app := &App{tasksDir: tasksDir, Executor: executorFunc(func(_ context.Context, _ Command) (CommandResult, *ExecuteError) {
		return CommandResult{}, opErr(ExecuteStart, "systemctl missing")
	})}
	var out bytes.Buffer
	if err := cmdStatus(context.Background(), app, &out); err != nil || !strings.Contains(out.String(), "running?") {
		t.Fatalf("cmdStatus failed or missing running?: err=%v out=%s", err, out.String())
	}
}

// TestCmdNotesExplicitEmptyArg verifies that an explicit empty argument is not consumed as repo.
func TestCmdNotesExplicitEmptyArg(t *testing.T) {
	var notesArgs []string
	app := &App{Executor: executorFunc(func(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
		argStr := strings.Join(cmd.Args, " ")
		if strings.Contains(argStr, "show-toplevel") && cmd.Stdout != nil {
			_, _ = io.WriteString(cmd.Stdout, "/home/dev/agents/worktrees/my-awesome-repo/task-456\n")
		} else if strings.Contains(argStr, "git-common-dir") && cmd.Stdout != nil {
			_, _ = io.WriteString(cmd.Stdout, "/home/dev/agents/worktrees/my-awesome-repo/.git\n")
		} else if strings.Contains(argStr, "notes") {
			notesArgs = cmd.Args
		}
		return CommandResult{ExitCode: 0}, nil
	})}
	var out bytes.Buffer
	if err := app.cmdNotes(context.Background(), []string{""}, &out); err != nil {
		t.Fatalf("cmdNotes failed: %v", err)
	}
	if !slices.Contains(notesArgs, "") {
		t.Fatalf("expected empty string argument to be passed to bus notes, got: %v", notesArgs)
	}
}

// executorFunc adapts a function to the Executor interface.
type executorFunc func(ctx context.Context, cmd Command) (CommandResult, *ExecuteError)

// Execute calls the underlying function.
func (f executorFunc) Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	return f(ctx, cmd)
}
