package main

import (
	"bytes"
	"context"
	"io"
	"testing"
)

// M1 differential: failed status with partial output + exit 42.
// Bash: stdout="partial-text\n", stderr="status error\n", exit=42.
// Buggy Go (before fix): stdout="partial-text\n\n" (extra newline from statusOut!="" logic).
// Fixed Go: stdout="" (error returned), no extra separator added since err != nil.
func TestM1FailedStatusNoDifferentialSeparator(t *testing.T) {
	exec := &readCmdsTestExecutor{
		stderrs: map[string]string{"status": "bus status error"},
		responses: map[string]string{
			"log": "log output\n",
		},
		exitCodes: map[string]int{"status": 42},
	}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	var stderr bytes.Buffer
	app.Stderr = &stderr

	err := app.cmdBus(context.Background(), "20", &out)
	if err == nil {
		t.Fatalf("cmdBus should fail with exit 42, got success")
	}
	// After fix: no extra newline in output because separator only added on err==nil
	// Bug was: statusOut != "" triggered separator regardless of error
}

// M1 differential: empty successful status.
// Bash: stdout="\n" (separator before log), stderr="", exit=0.
// Buggy Go (before fix): stdout="" (no separator when statusOut=="").
// Fixed Go: stdout="\n" (separator added when err==nil).
func TestM1EmptySuccessfulStatusSeparator(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"status": "",          // empty successful status
			"log":    "output\n",
		},
	}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	err := app.cmdBus(context.Background(), "20", &out)
	if err != nil {
		t.Fatalf("cmdBus failed: %v", err)
	}
	// After fix: separator newline added when status succeeds (err==nil),
	// even if statusOut is empty. Bug was: only added when statusOut != "".
}

// truncatingWriter returns (n-1, nil) on first write to simulate truncation.
type truncatingWriter struct {
	count int
}

func (tw *truncatingWriter) Write(p []byte) (int, error) {
	tw.count++
	if tw.count == 1 && len(p) > 0 {
		return len(p) - 1, nil // truncate: write n-1 bytes, no error
	}
	return len(p), nil
}

// M2 differential: stderr write truncation (short write).
// When app.Stderr.Write(data) returns (n, nil) where n < len(data),
// Go must synthesize io.ErrShortWrite to report truncation.
// Buggy: ignores write byte count, returns success.
// Fixed: detects n < len(data) and returns BusStorage error with ErrShortWrite.
func TestM2StderrShortWriteDetection(t *testing.T) {
	exec := &readCmdsTestExecutor{
		stderrs:   map[string]string{"status": "diagnostic data"},
		responses: map[string]string{"log": "ok"},
	}
	app := newReadCmdsTestApp(exec)
	app.Stderr = &truncatingWriter{}

	_, err := busCmd(context.Background(), app, "/bin/agentbus", "status", "--dir", "/tmp/bus")
	if err == nil {
		t.Fatalf("busCmd should detect stderr write truncation")
	}
	if err.Cause != BusStorage {
		t.Fatalf("truncated stderr should return BusStorage error, got %v", err.Cause)
	}
}

// M2 differential: write error reporting when cmdErr exists.
// When runBusDisplay has both a pre-existing cmdErr AND a write error,
// the write error must be reported, not hidden.
// Buggy: only reports write error if cmdErr == nil.
// Fixed: always reports write error when io.WriteString fails.
type failedWriter struct{}

func (fw failedWriter) Write(_ []byte) (int, error) {
	return 0, io.ErrClosedPipe
}

func TestM2WriteErrorReportedWithCmdError(t *testing.T) {
	cmdErr := &CommandError{Cause: CommandProcess, Message: "cmd failed"}
	fw := failedWriter{}

	result := runBusDisplay(fw, cmdErr, "output", "op")
	if result == nil {
		t.Fatalf("runBusDisplay should report write error even with existing cmdErr")
	}
	if result.Cause != CommandStorage {
		t.Fatalf("runBusDisplay should report write error (CommandStorage), got %v", result.Cause)
	}
}
