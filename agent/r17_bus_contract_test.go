package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"testing"
)

// TestCmdBusFailedStatusExactOutputAndRawExit verifies that a failed status command preserves partial stdout without extra separator, replays stderr, and reports exit 42.
func TestCmdBusFailedStatusExactOutputAndRawExit(t *testing.T) {
	exec := executorFunc(func(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stdout != nil {
			if _, err := cmd.Stdout.Write([]byte("partial-text\n")); err != nil {
				t.Fatalf("write stdout: %v", err)
			}
		}
		if cmd.Stderr != nil {
			if _, err := cmd.Stderr.Write([]byte("status-error\n")); err != nil {
				t.Fatalf("write stderr: %v", err)
			}
		}
		return CommandResult{ExitCode: 42}, nil
	})
	var stdout, stderr bytes.Buffer
	app := &App{Executor: exec, Stderr: &stderr}
	err := app.cmdBus(context.Background(), "20", &stdout)
	if err == nil {
		t.Fatalf("expected error from failed status, got nil")
	}
	if stdout.String() != "partial-text\n" {
		t.Fatalf("stdout mismatch: got %q, want %q", stdout.String(), "partial-text\n")
	}
	if stderr.String() != "status-error\n" {
		t.Fatalf("stderr mismatch: got %q, want %q", stderr.String(), "status-error\n")
	}
	var raw exitStatusError
	if !errors.As(err, &raw) || raw.code != 42 {
		t.Fatalf("expected raw exit 42, got %v (err=%v)", raw, err)
	}
}

// TestRunBusDisplayWriteErrorClosedPipeSentinel verifies that a write failure during delegated error returns CommandStorage wrapping io.ErrClosedPipe.
func TestRunBusDisplayWriteErrorClosedPipeSentinel(t *testing.T) {
	cmdErr := &CommandError{Cause: CommandProcess, Message: "delegated command failed"}
	err := runBusDisplay(&closedPipeWriter{}, cmdErr, "partial output", "bus display")
	if err == nil || err.Cause != CommandStorage || !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("expected CommandStorage wrapping io.ErrClosedPipe, got %v", err)
	}
}

// TestBusCmdStderrShortWriteSentinel verifies that truncated stderr replay returns BusStorage wrapping io.ErrShortWrite.
func TestBusCmdStderrShortWriteSentinel(t *testing.T) {
	exec := executorFunc(func(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Stderr != nil {
			if _, err := cmd.Stderr.Write([]byte("diagnostic detail\n")); err != nil {
				t.Fatalf("write stderr: %v", err)
			}
		}
		return CommandResult{ExitCode: 0}, nil
	})
	app := &App{Executor: exec, Stderr: &failingWriter{buf: &bytes.Buffer{}}}
	_, err := busCmd(context.Background(), app, "agentbus", "status")
	if err == nil || err.Cause != BusStorage || !errors.Is(err, io.ErrShortWrite) {
		t.Fatalf("expected BusStorage wrapping io.ErrShortWrite, got %v", err)
	}
}
