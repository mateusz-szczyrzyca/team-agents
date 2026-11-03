package main

import (
	"bytes"
	"io"
	"os"
	"testing"
)

// TestEmptyArgv verifies that with no arguments, help is displayed.
func TestEmptyArgv(t *testing.T) {
	old := os.Stdout
	r, w, _ := os.Pipe()
	os.Stdout = w

	err := run([]string{})

	if closeErr := w.Close(); closeErr != nil {
		t.Logf("error closing pipe: %v", closeErr)
	}
	os.Stdout = old
	out, _ := io.ReadAll(r)

	if err != nil {
		t.Errorf("run([]) returned error: %v", err)
	}
	if len(out) == 0 {
		t.Error("run([]) produced no output")
	}
	if !bytes.Contains(out, []byte("Usage:")) {
		t.Errorf("output does not contain Usage: %s", string(out))
	}
}

// TestHelpFlag verifies that -h flag shows help.
func TestHelpFlag(t *testing.T) {
	old := os.Stdout
	r, w, _ := os.Pipe()
	os.Stdout = w

	err := run([]string{"-h"})

	if closeErr := w.Close(); closeErr != nil {
		t.Logf("error closing pipe: %v", closeErr)
	}
	os.Stdout = old
	out, _ := io.ReadAll(r)

	if err != nil {
		t.Errorf("run([\"-h\"]) returned error: %v", err)
	}
	if !bytes.Contains(out, []byte("Usage:")) {
		t.Errorf("output does not contain Usage: %s", string(out))
	}
}

// TestHelpLongFlag verifies that --help flag shows help.
func TestHelpLongFlag(t *testing.T) {
	old := os.Stdout
	r, w, _ := os.Pipe()
	os.Stdout = w

	err := run([]string{"--help"})

	if closeErr := w.Close(); closeErr != nil {
		t.Logf("error closing pipe: %v", closeErr)
	}
	os.Stdout = old
	out, _ := io.ReadAll(r)

	if err != nil {
		t.Errorf("run([\"--help\"]) returned error: %v", err)
	}
	if !bytes.Contains(out, []byte("Usage:")) {
		t.Errorf("output does not contain Usage: %s", string(out))
	}
}

// TestHelpCommand verifies that help command shows help.
func TestHelpCommand(t *testing.T) {
	old := os.Stdout
	r, w, _ := os.Pipe()
	os.Stdout = w

	err := run([]string{"help"})

	if closeErr := w.Close(); closeErr != nil {
		t.Logf("error closing pipe: %v", closeErr)
	}
	os.Stdout = old
	out, _ := io.ReadAll(r)

	if err != nil {
		t.Errorf("run([\"help\"]) returned error: %v", err)
	}
	if !bytes.Contains(out, []byte("Usage:")) {
		t.Errorf("output does not contain Usage: %s", string(out))
	}
}

// TestUnknownCommand verifies that unknown commands return an error.
func TestUnknownCommand(t *testing.T) {
	err := run([]string{"unknowncommand"})
	if err == nil {
		t.Error("run([\"unknowncommand\"]) should return an error")
	}
	errMsg := err.Error()
	if errMsg != "unknown command: unknowncommand" {
		t.Errorf("unexpected error message: %s", errMsg)
	}
}

// TestTypedNilError verifies that errors returned are never typed-nil pointers.
func TestTypedNilError(t *testing.T) {
	err := run([]string{"help"})
	if err != nil {
		t.Fatalf("run([\"help\"]) should not return error, got: %v", err)
	}
	// Verify nil is truly nil, not a typed pointer
	if err != nil {
		t.Fatal("nil error check failed")
	}
}
