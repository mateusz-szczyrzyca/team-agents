package main

import (
	"bytes"
	"context"
	"io"
	"testing"
)

// failingWriter returns (n-1, nil) to simulate a short write without error.
type failingWriter struct {
	buf       *bytes.Buffer
	callCount int
}

func (fw *failingWriter) Write(p []byte) (int, error) {
	fw.callCount++
	if fw.callCount == 1 && len(p) > 0 {
		// Simulate truncation: write all but last byte, return success
		n, _ := fw.buf.Write(p[:len(p)-1])
		return n, nil
	}
	return fw.buf.Write(p)
}

// closedPipeWriter always returns a closed pipe error on first write.
type closedPipeWriter struct {
	callCount int
}

func (cpw *closedPipeWriter) Write(p []byte) (int, error) {
	cpw.callCount++
	if cpw.callCount == 1 {
		return 0, io.ErrClosedPipe
	}
	return len(p), nil
}

// TestCmdBusEmptySuccessfulStatusHasSeparator tests that empty successful status gets newline separator.
func TestCmdBusEmptySuccessfulStatusHasSeparator(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"status": "", // empty successful status
			"log":    "event1\n",
		},
	}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	err := app.cmdBus(context.Background(), "20", &out)
	if err != nil {
		t.Fatalf("cmdBus failed: %v", err)
	}
	output := out.String()
	// When status succeeds (empty or not), there should be a separator newline before log
	// Current buggy behavior: if statusOut == "", no newline is added
	// So output would be "event1\n" instead of "\nevent1\n"
	expected := "\nevent1\n"
	if output != expected {
		t.Fatalf("cmdBus output mismatch:\n  got: %q\n  want: %q", output, expected)
	}
}

// TestRunBusDisplayWriteErrorWithCmdError tests that write error is reported even when cmdErr is non-nil.
func TestRunBusDisplayWriteErrorWithCmdError(t *testing.T) {
	cmdErr := &CommandError{Cause: CommandProcess, Message: "command failed"}
	failWriter := &closedPipeWriter{}

	result := runBusDisplay(failWriter, cmdErr, "some output", "test op")

	// Should return an error (not just cmdErr, but the write error)
	if result == nil {
		t.Fatalf("runBusDisplay should return an error when write fails, even with existing cmdErr")
	}

	// The error should be CommandStorage (from write failure), not just the original cmdErr
	if result.Cause != CommandStorage {
		t.Fatalf("runBusDisplay should return CommandStorage error for write failure, got %v", result.Cause)
	}
}

// TestBusCmdStderrShortWrite tests that busCmd detects when stderr write is truncated.
func TestBusCmdStderrShortWrite(t *testing.T) {
	// Create a test executor that returns stderr
	// Don't include "status" in responses; only in stderrs to ensure stderr path is taken
	exec := &readCmdsTestExecutor{
		responses: map[string]string{"log": "ok"},
		stderrs:   map[string]string{"status": "diagnostic output"}, // stderr to be written for status command
	}

	app := newReadCmdsTestApp(exec)
	// Use a writer that truncates on first write
	app.Stderr = &failingWriter{buf: &bytes.Buffer{}}

	// busCmd should detect the truncation and return an error
	// Currently it ignores the write byte count and returns success
	_, err := busCmd(context.Background(), app, "/bin/agentbus", "status", "--dir", "/tmp/bus")

	// Should get an error about truncated write
	if err == nil {
		t.Fatalf("busCmd should return error when stderr write is truncated")
	}

	// Error should indicate I/O failure
	if err.Cause != BusStorage {
		t.Fatalf("busCmd should return BusStorage error for write truncation, got %v", err.Cause)
	}
}
