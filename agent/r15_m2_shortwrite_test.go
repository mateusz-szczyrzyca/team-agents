package main

import (
	"context"
	"io"
	"testing"
)

// shortWriteHelper returns (n-1, nil) to simulate truncated write without error.
type shortWriteHelper struct {
	callCount int
}

func (sw *shortWriteHelper) Write(p []byte) (int, error) {
	sw.callCount++
	if sw.callCount == 1 && len(p) > 0 {
		// First write: return less than requested, no error (truncation without error)
		return len(p) - 1, nil
	}
	// Subsequent writes succeed normally
	return len(p), nil
}

// TestM2BusCmdDetectsStderrShortWrite verifies busCmd detects incomplete stderr writes.
// M2 issue: when stderr write returns (n, nil) where n < len(data), busCmd should
// synthesize io.ErrShortWrite to report the truncation. Current code ignores byte count.
func TestM2BusCmdDetectsStderrShortWrite(t *testing.T) {
	// Create executor that produces stderr but succeeds
	exec := &readCmdsTestExecutor{
		stderrs: map[string]string{
			"status": "diagnostic data", // stderr that will be written
		},
		responses: map[string]string{
			"log": "ok",
		},
	}

	app := newReadCmdsTestApp(exec)
	// Install a writer that truncates on first call
	app.Stderr = &shortWriteHelper{}

	// busCmd should detect that stderr write was truncated
	_, err := busCmd(context.Background(), app, "/bin/agentbus", "status", "--dir", "/tmp/bus")

	// Should get an error about the truncated write
	if err == nil {
		t.Fatalf("busCmd should return error when stderr write is truncated (short write)")
	}

	// Error should be BusStorage (I/O failure category)
	if err.Cause != BusStorage {
		t.Fatalf("busCmd truncated write should return BusStorage error, got %v", err.Cause)
	}

	// The error should reference I/O truncation
	if err.Message == "" {
		t.Fatalf("busCmd truncated write error should have a message")
	}
}

// closedWriter implements io.Writer and always returns ErrClosedPipe.
type closedWriter struct{}

func (cw closedWriter) Write(p []byte) (int, error) {
	return 0, io.ErrClosedPipe
}

// TestM2RunBusDisplayReportsWriteErrorWithCmdError tests write error is not hidden by cmdErr.
func TestM2RunBusDisplayReportsWriteErrorWithCmdError(t *testing.T) {
	cmdErr := &CommandError{Cause: CommandProcess, Message: "command failed"}
	fw := closedWriter{}

	result := runBusDisplay(fw, cmdErr, "some text", "op")

	if result == nil {
		t.Fatalf("runBusDisplay must return error when write fails, even with existing cmdErr")
	}

	// Error should be about the write, not the command
	if result.Cause != CommandStorage {
		t.Fatalf("runBusDisplay should report write error (CommandStorage), got %v", result.Cause)
	}
}
