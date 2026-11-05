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

func TestDispatchRecognizesR09Commands(t *testing.T) {
	commands := []string{"status", "ls", "show", "agents", "_agents-json", "ack", "bus", "inv", "notes", "helpers"}
	for _, cmd := range commands {
		var stdout, stderr bytes.Buffer
		err := dispatch(cmd, nil, &stdout, &stderr)
		if err != nil && err.Cause == CommandUnknownCommand {
			t.Fatalf("dispatch(%q) returned CommandUnknownCommand, want recognized command", cmd)
		}
	}
}

func TestDispatchShowMissingArg(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("show", nil, &stdout, &stderr)
	if err == nil || err.Cause != CommandInvalidArgument {
		t.Fatalf("dispatch(show, nil) err = %v, want CommandInvalidArgument", err)
	}
}

func TestDispatchWithArgs(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("show", []string{"nonexistent-task"}, &stdout, &stderr)
	if err == nil || err.Cause != CommandInvalidArgument {
		t.Fatalf("dispatch(show, [nonexistent-task]) err = %v, want CommandInvalidArgument", err)
	}
}

func TestDispatchAckMissingArg(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("ack", nil, &stdout, &stderr)
	if err == nil || err.Cause != CommandInvalidArgument {
		t.Fatalf("dispatch(ack, nil) err = %v, want CommandInvalidArgument", err)
	}
}

func TestDispatchAgentsJSON(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("_agents-json", nil, &stdout, &stderr)
	if err != nil {
		t.Fatalf("dispatch(_agents-json, nil) err = %v, want nil", err)
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
