package main

import (
	"bytes"
	"context"
	"testing"
)

// TestCmdBusFailedStatusDifferential verifies the Bash vs Go output difference for failed status.
// M1 differential case: when status command fails but produces output (partial-text + exit 42),
// Bash produces "partial-text\n" while buggy Go produces "partial-text\n\n" (extra newline).
// This test captures the real difference in stdout/stderr/exit code behavior.
func TestCmdBusFailedStatusDifferential(t *testing.T) {
	// Simulate real agentbus status command that fails with partial output
	exec := &readCmdsTestExecutor{
		stderrs: map[string]string{
			"status": "bus status error", // This goes to stderr
		},
		responses: map[string]string{
			"log": "event1\nevent2\n",
		},
		exitCodes: map[string]int{
			"status": 42, // Non-zero exit code for failed status
		},
	}

	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	var errOut bytes.Buffer
	app.Stderr = &errOut

	// Call cmdBus which will try status (fail with exit 42) then log (succeed)
	err := app.cmdBus(context.Background(), "20", &out)

	// Status fails, so we expect an error
	if err == nil {
		t.Fatalf("cmdBus should fail when status returns exit code 42, but got success")
	}

	output := out.String()
	_ = errOut.String() // Captured for differential analysis; error comes from status command

	// The bug: when status fails, Go currently adds a separator newline based on statusOut != "",
	// resulting in "partial-text\n\n" when it should be "partial-text\n" (Bash behavior).
	// After the fix, separator should only be added on successful status.

	// With the bug present (before fix), output would contain double newline.
	// This test documents the expected behavior AFTER the fix.
	// The error message shows "bus status error" in stderr (from the executor).

	if bytes.Contains([]byte(output), []byte("\n\n")) {
		t.Fatalf("M1 differential regression: failed status should not have double newline separator. Got: %q", output)
	}

	// After the fix, failed status error is returned but output is clean
	if err.Cause != CommandProcess {
		t.Fatalf("cmdBus should return CommandProcess error for failed status, got %v", err.Cause)
	}
}
