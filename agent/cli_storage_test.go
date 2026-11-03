package main

import (
	"errors"
	"io"
	"testing"
)

// failedUsageWriter injects a standard-library writer failure without touching stdout.
type failedUsageWriter struct {
	failure error // exact failure to retain in the typed chain
}

// Write implements io.Writer for deterministic output failure tests.
func (w failedUsageWriter) Write(_ []byte) (int, error) { return 0, w.failure }

// TestUsageWriteFailureIsStorage checks the help and unknown-command error branches.
func TestUsageWriteFailureIsStorage(t *testing.T) {
	// underlying stands in for a closed stdout pipe.
	underlying := errors.New("closed output")
	// writer injects the same identifiable error on every write.
	writer := failedUsageWriter{failure: underlying}
	for _, cmd := range []string{"help", "-h", "--help", "unknown-cmd"} {
		t.Run(cmd, func(t *testing.T) {
			// failure must prioritize the failed usage write over command classification.
			failure := dispatch(cmd, nil, writer, io.Discard)
			if failure == nil {
				t.Fatal("dispatch returned nil")
			}
			if failure.Cause != CommandStorage || failure.Class() != ClassStorage || !errors.Is(failure, ErrStorage) || !errors.Is(failure, underlying) {
				t.Fatalf("dispatch returned cause %v, want Storage retaining write failure", failure.Cause)
			}
			// storage confirms dispatch converted a concrete lower-level failure.
			var storage *StorageError
			if !errors.As(failure, &storage) {
				t.Fatal("missing concrete StorageError")
			}
			if code := run([]string{cmd}, writer, io.Discard); code != 1 {
				t.Fatalf("exit = %d", code)
			}
		})
	}
	if code := run(nil, writer, io.Discard); code != 1 {
		t.Fatalf("empty argv exit = %d", code)
	}
	// failure also verifies the low-level usage writer classifies output failures.
	failure := printUsage(writer)
	if failure == nil || !errors.Is(failure, ErrStorage) || !errors.Is(failure, underlying) {
		t.Fatal("printUsage lost storage classification")
	}
}
