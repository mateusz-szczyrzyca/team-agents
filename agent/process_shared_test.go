package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

// TestExecuteSharedStreamsComplete checks both streams through one unsynchronized buffer.
func TestExecuteSharedStreamsComplete(t *testing.T) {
	var shared bytes.Buffer
	cmd := helperCmd("write-distinct-streams")
	cmd.Stdout, cmd.Stderr = &shared, &shared
	result, err := (OSExecutor{}).Execute(context.Background(), cmd)
	if err != nil || result.ExitCode != 0 {
		t.Fatalf("Execute = %+v, %v", result, err)
	}
	output := shared.String()
	if len(output) != 2*32*4096 || strings.Count(output, "O") != 32*4096 || strings.Count(output, "E") != 32*4096 {
		t.Fatalf("incomplete streams: length=%d, stdout=%d, stderr=%d", len(output), strings.Count(output, "O"), strings.Count(output, "E"))
	}
}

// nonComparableBufferWriter wraps a bytes.Buffer with a slice, making it non-comparable.
type nonComparableBufferWriter struct {
	buf  *bytes.Buffer // underlying buffer
	tags []string      // non-comparable slice field
}

// Write writes to the underlying buffer.
func (w nonComparableBufferWriter) Write(p []byte) (int, error) {
	return w.buf.Write(p)
}

// TestExecuteNonComparableSharedWriter verifies that assigning the same non-comparable writer
// to both stdout and stderr serializes writes without data races or stream truncation.
func TestExecuteNonComparableSharedWriter(t *testing.T) {
	var shared bytes.Buffer
	writer := nonComparableBufferWriter{buf: &shared, tags: []string{"tag"}}
	cmd := helperCmd("write-distinct-streams")
	cmd.Stdout, cmd.Stderr = writer, writer
	result, err := (OSExecutor{}).Execute(context.Background(), cmd)
	if err != nil || result.ExitCode != 0 {
		t.Fatalf("Execute = %+v, %v", result, err)
	}
	output := shared.String()
	if len(output) != 2*32*4096 || strings.Count(output, "O") != 32*4096 || strings.Count(output, "E") != 32*4096 {
		t.Fatalf("incomplete streams: length=%d, stdout=%d, stderr=%d", len(output), strings.Count(output, "O"), strings.Count(output, "E"))
	}
}
