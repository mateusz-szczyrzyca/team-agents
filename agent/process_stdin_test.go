package main

import (
	"context"
	"errors"
	"io"
	"testing"
)

// failReader returns a deterministic error to the command's stdin copier.
type failReader struct {
	err error // error returned on every read
}

// Read reports the configured input failure without sending bytes.
func (f failReader) Read(_ []byte) (int, error) { return 0, f.err }

// TestExecuteStdinReaderFailure retains input errors when the child exits 42.
func TestExecuteStdinReaderFailure(t *testing.T) {
	cmd := helperCmd("consume-stdin-exit")
	cmd.Stdin = failReader{err: io.ErrUnexpectedEOF}
	result, err := (OSExecutor{}).Execute(context.Background(), cmd)
	if err == nil {
		t.Fatalf("expected stdin failure, got result %+v", result)
	}
	if err.Cause != ExecuteWait {
		t.Fatalf("expected ExecuteWait, got %v", err.Cause)
	}
	if !errors.Is(err, ErrProcess) || !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatalf("expected Process error retaining io.ErrUnexpectedEOF, got %v", err)
	}
}
