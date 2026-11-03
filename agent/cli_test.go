package main

import (
	"bytes"
	"errors"
	"strings"
	"testing"
)

func TestEmptyArgvRunsHelpWithExitZero(t *testing.T) {
	var stdout, stderr bytes.Buffer
	code := run(nil, &stdout, &stderr)
	if code != 0 {
		t.Fatalf("run(nil) exit code = %d, want 0", code)
	}
	if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
		t.Fatalf("run(nil) stdout does not contain usage: %q", stdout.String())
	}
}

func TestHelpAliasesRunHelpWithExitZero(t *testing.T) {
	aliases := []string{"help", "-h", "--help"}
	for _, alias := range aliases {
		var stdout, stderr bytes.Buffer
		code := run([]string{alias}, &stdout, &stderr)
		if code != 0 {
			t.Fatalf("run([%q]) exit code = %d, want 0", alias, code)
		}
		if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
			t.Fatalf("run([%q]) stdout does not contain usage: %q", alias, stdout.String())
		}
	}
}

func TestUnknownCommandExitsOneWithUsage(t *testing.T) {
	var stdout, stderr bytes.Buffer
	code := run([]string{"unknown-cmd"}, &stdout, &stderr)
	if code != 1 {
		t.Fatalf("run([\"unknown-cmd\"]) exit code = %d, want 1", code)
	}
	if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
		t.Fatalf("run([\"unknown-cmd\"]) stdout does not contain usage: %q", stdout.String())
	}
}

func TestDispatchUnknownCommand(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("unknown-cmd", nil, &stdout, &stderr)
	if err == nil {
		t.Fatal("dispatch(\"unknown-cmd\") returned nil error, want CommandUnknownCommand")
	}
	if err.Cause != CommandUnknownCommand {
		t.Fatalf("dispatch(\"unknown-cmd\") cause = %v, want CommandUnknownCommand", err.Cause)
	}
	if !errors.Is(err, ErrInvalidArgument) {
		t.Fatalf("dispatch(\"unknown-cmd\") error does not unwrap to ErrInvalidArgument")
	}
}

func TestDispatchHelp(t *testing.T) {
	for _, cmd := range []string{"help", "-h", "--help"} {
		var stdout, stderr bytes.Buffer
		err := dispatch(cmd, nil, &stdout, &stderr)
		if err != nil {
			t.Fatalf("dispatch(%q) returned error = %v, want nil", cmd, err)
		}
		if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
			t.Fatalf("dispatch(%q) stdout does not contain usage", cmd)
		}
	}
}
