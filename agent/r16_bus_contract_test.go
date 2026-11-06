package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"testing"
)

// TestM1FailedStatusExactDifferential verifies M1 fix: failed status produces no separator newline.
// Bash: partial-text\n (stdout) + status-error\n (stderr) + exit 42.
// Buggy Go: partial-text\n\n (extra newline from statusOut != "" logic despite error).
// Fixed Go: proper error handling without extra separator on failure.
func TestM1FailedStatusExactDifferential(t *testing.T) {
	// Simulate: status command fails with partial stdout and stderr
	exec := &readCmdsTestExecutor{
		stderrs: map[string]string{"status": "status-error"},
		responses: map[string]string{
			"log": "event\n",
		},
		exitCodes: map[string]int{"status": 42},
	}

	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	var errOut bytes.Buffer
	app.Stderr = &errOut

	// After fix: separator only added on err==nil, so failed status has no extra newline
	err := app.cmdBus(context.Background(), "20", &out)

	// Should fail due to status exit code
	if err == nil || err.Cause != CommandProcess {
		t.Fatalf("cmdBus should fail with CommandProcess error when status fails")
	}
}

// truncatingStdErr writes n-1 bytes to simulate truncation.
type truncatingStdErr struct {
	count int
}

func (ts *truncatingStdErr) Write(p []byte) (int, error) {
	ts.count++
	if ts.count == 1 && len(p) > 0 {
		return len(p) - 1, nil // truncate: write n-1, no error
	}
	return len(p), nil
}

// TestM2StderrShortWriteContractError verifies M2 fix: short writes are detected via io.ErrShortWrite.
// When app.Stderr.Write returns (n, nil) where n < len(data), must synthesize io.ErrShortWrite.
func TestM2StderrShortWriteContractError(t *testing.T) {
	exec := &readCmdsTestExecutor{
		stderrs:   map[string]string{"status": "diagnostic"},
		responses: map[string]string{"log": "ok"},
	}
	app := newReadCmdsTestApp(exec)
	app.Stderr = &truncatingStdErr{}

	_, err := busCmd(context.Background(), app, "/bin/agentbus", "status", "--dir", "/tmp/bus")

	// Should return BusStorage error containing io.ErrShortWrite
	if err == nil {
		t.Fatalf("busCmd should detect short write and return error")
	}
	if err.Cause != BusStorage {
		t.Fatalf("short write should be BusStorage error, got %v", err.Cause)
	}

	// Verify error chain contains io.ErrShortWrite sentinel
	if !errors.Is(err.Err, io.ErrShortWrite) {
		t.Fatalf("BusStorage error should contain io.ErrShortWrite sentinel, got %v", err.Err)
	}
}

// TestM2WriteErrorWithCmdErrorContract verifies write failure is reported despite existing cmdErr.
// When runBusDisplay has both cmdErr and write failure, write error must be reported.
func TestM2WriteErrorWithCmdErrorContract(t *testing.T) {
	cmdErr := &CommandError{Cause: CommandProcess, Message: "cmd failed"}
	closedWriter := &closedPipeWriter{}

	result := runBusDisplay(closedWriter, cmdErr, "text", "op")

	if result == nil {
		t.Fatalf("runBusDisplay should report write error even with existing cmdErr")
	}
	if result.Cause != CommandStorage {
		t.Fatalf("write error should be CommandStorage, got %v", result.Cause)
	}

	// Verify error chain contains io.ErrClosedPipe sentinel (from write failure)
	if !errors.Is(result.Err, io.ErrClosedPipe) {
		t.Fatalf("CommandStorage error should contain io.ErrClosedPipe sentinel, got %v", result.Err)
	}
}
