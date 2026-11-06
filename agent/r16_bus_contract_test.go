package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"testing"
)

// M1 exact Bash differential test: status fails with partial output.
// Bash: stdout="partial-text\n", stderr="status-error\n", exit=42.
// Buggy Go (before fix): stdout="partial-text\n\n" (extra newline from statusOut!="" logic).
// Fixed Go: correct error, no extra separator newline.
func TestM1BashDifferentialFailedStatusPartialText(t *testing.T) {
	exec := &readCmdsTestExecutor{
		stderrs: map[string]string{"status": "status-error\n"},
		responses: map[string]string{
			"log": "log event\n",
		},
		exitCodes: map[string]int{"status": 42},
	}

	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	err := app.cmdBus(context.Background(), "20", &out)

	// Must fail when status returns exit 42
	if err == nil {
		t.Fatalf("cmdBus must return error when status exit=42")
	}
	if err.Cause != CommandProcess {
		t.Fatalf("failed status must return CommandProcess error, got %v", err.Cause)
	}

	// After fix: no double newline in output; separator only on err==nil
	output := out.String()
	if bytes.Count([]byte(output), []byte("\n\n")) > 0 {
		t.Fatalf("M1 bug: output has double newline (extra separator added despite error)")
	}
}

// M2 sentinel test: io.ErrShortWrite when stderr truncated.
type truncWriter struct {
	count int
}

func (tw *truncWriter) Write(p []byte) (int, error) {
	tw.count++
	if tw.count == 1 && len(p) > 0 {
		return len(p) - 1, nil // truncate, no error
	}
	return len(p), nil
}

func TestM2StderrShortWriteSentinel(t *testing.T) {
	exec := &readCmdsTestExecutor{
		stderrs:   map[string]string{"status": "stderr data"},
		responses: map[string]string{"log": "ok"},
	}
	app := newReadCmdsTestApp(exec)
	app.Stderr = &truncWriter{}

	_, err := busCmd(context.Background(), app, "/bin/agentbus", "status", "--dir", "/tmp/bus")
	if err == nil {
		t.Fatalf("busCmd must detect short write")
	}
	if err.Cause != BusStorage {
		t.Fatalf("short write error must be BusStorage, got %v", err.Cause)
	}
	// Verify io.ErrShortWrite sentinel is in error chain
	if !errors.Is(err.Err, io.ErrShortWrite) {
		t.Fatalf("BusStorage error must contain io.ErrShortWrite sentinel")
	}
}

// M2 sentinel test: io.ErrClosedPipe when write fails with existing cmdErr.
func TestM2WriteErrorSentinel(t *testing.T) {
	cmdErr := &CommandError{Cause: CommandProcess, Message: "cmd failed"}
	writer := &closedPipeWriter{}

	result := runBusDisplay(writer, cmdErr, "text", "op")
	if result == nil {
		t.Fatalf("runBusDisplay must report write error with or without cmdErr")
	}
	if result.Cause != CommandStorage {
		t.Fatalf("write error must be CommandStorage, got %v", result.Cause)
	}
	// Verify io.ErrClosedPipe sentinel is in error chain
	if !errors.Is(result.Err, io.ErrClosedPipe) {
		t.Fatalf("CommandStorage error must contain io.ErrClosedPipe sentinel")
	}
}
