package main

import (
	"context"
	"testing"
)

// nestedWriter has a comparable static type whose interface field may not be comparable.
type nestedWriter struct {
	payload any // payload exposes nested dynamic comparability during writer identity checks
}

// Write discards output so the test isolates writer identity from I/O behavior.
func (nestedWriter) Write(p []byte) (int, error) {
	return len(p), nil
}

// TestExecuteWriterWithUncomparableField requires identity checks to inspect dynamic fields.
func TestExecuteWriterWithUncomparableField(t *testing.T) {
	defer func() {
		if recovered := recover(); recovered != nil {
			t.Fatalf("Execute panicked for a writer containing a slice: %v", recovered)
		}
	}()

	writer := nestedWriter{payload: []int{1}}
	cmd := helperCmd("echo-args", "nested")
	cmd.Stdout = writer
	cmd.Stderr = writer
	result, err := (OSExecutor{}).Execute(context.Background(), cmd)
	if err != nil {
		t.Fatalf("Execute failed: %v", err)
	}
	if result.ExitCode != 0 {
		t.Fatalf("exit code = %d, want 0", result.ExitCode)
	}
}
