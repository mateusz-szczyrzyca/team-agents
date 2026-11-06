package main

import (
	"io"
	"os"
	"path/filepath"
	"testing"
)

// writeAckFixture creates inert metadata in an isolated task root.
func writeAckFixture(t *testing.T, root, id, status string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(root, id), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, id, "meta.env"), []byte("ID="+id+"\nSTATUS="+status+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
}

// ackFailWriter exercises stdout errors without any external process.
type ackFailWriter struct{}

// Write returns a sentinel for assertions through the typed storage wrapper.
func (ackFailWriter) Write([]byte) (int, error) { return 0, io.ErrClosedPipe }
