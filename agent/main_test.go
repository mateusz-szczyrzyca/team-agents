// main_test.go tests the CLI skeleton and error handling.
package main

import (
	"bytes"
	"strings"
	"testing"
)

// TestEmptyArgv tests that empty arguments result in no error (shows status/help).
func TestEmptyArgv(t *testing.T) {
	err := dispatchCommand([]string{})
	if err != nil {
		t.Errorf("dispatchCommand([]string{}) = %v, want nil", err)
	}
}

// TestHelpAliasH tests that -h shows help without error.
func TestHelpAliasH(t *testing.T) {
	err := dispatchCommand([]string{"-h"})
	if err != nil {
		t.Errorf("dispatchCommand([]string{\"-h\"}) = %v, want nil", err)
	}
}

// TestHelpAliasLongHelp tests that --help shows help without error.
func TestHelpAliasLongHelp(t *testing.T) {
	err := dispatchCommand([]string{"--help"})
	if err != nil {
		t.Errorf("dispatchCommand([]string{\"--help\"}) = %v, want nil", err)
	}
}

// TestHelpCommand tests that help command shows help without error.
func TestHelpCommand(t *testing.T) {
	err := dispatchCommand([]string{"help"})
	if err != nil {
		t.Errorf("dispatchCommand([]string{\"help\"}) = %v, want nil", err)
	}
}

// TestUnknownCommand tests that unknown commands return RunUnknownCommand error.
func TestUnknownCommand(t *testing.T) {
	err := dispatchCommand([]string{"unknown"})
	if err == nil {
		t.Error("dispatchCommand([]string{\"unknown\"}) = nil, want error")
		return
	}
	if err.Cause() != RunUnknownCommand {
		t.Errorf("err.Cause() = %v, want RunUnknownCommand", err.Cause())
	}
}

// TestRunErrorString tests that RunError properly formats error messages.
func TestRunErrorString(t *testing.T) {
	err := NewRunError(RunUnknownCommand, "test message")
	expected := "test message"
	if err.Error() != expected {
		t.Errorf("RunError.Error() = %q, want %q", err.Error(), expected)
	}
}

// TestTypedNilError tests that a nil RunError has safe default behavior.
func TestTypedNilError(t *testing.T) {
	var err *RunError
	// err is a typed nil (not nil interface)

	// Accessing methods on nil should not panic
	if msg := err.Error(); msg != "" {
		t.Errorf("nil RunError.Error() = %q, want empty string", msg)
	}

	// Nil error should default to Storage cause
	if cause := err.Cause(); cause != RunStorage {
		t.Errorf("nil RunError.Cause() = %v, want RunStorage", cause)
	}
}

// TestHelpProviderIsAlias tests help alias recognition.
func TestHelpProviderIsAlias(t *testing.T) {
	hp := NewHelpProvider()
	tests := []struct {
		input    string
		expected bool
	}{
		{"-h", true},
		{"--help", true},
		{"help", true},
		{"status", false},
		{"unknown", false},
		{"", false},
	}

	for _, tt := range tests {
		if result := hp.IsHelpAlias(tt.input); result != tt.expected {
			t.Errorf("IsHelpAlias(%q) = %v, want %v", tt.input, result, tt.expected)
		}
	}
}

// TestUsageTextContainsCommands verifies the usage text is populated.
func TestUsageTextContainsCommands(t *testing.T) {
	usage := usageText()
	if usage == "" {
		t.Error("usageText() returned empty string")
		return
	}

	requiredStrings := []string{"Usage:", "Commands:", "run", "status", "agents"}
	for _, required := range requiredStrings {
		if !strings.Contains(usage, required) {
			t.Errorf("usageText() does not contain %q", required)
		}
	}
}

// TestAppCreation tests that the App structure can be created.
func TestAppCreation(t *testing.T) {
	stdout := &bytes.Buffer{}
	stderr := &bytes.Buffer{}
	app := NewApp(stdout, stderr)

	if app == nil {
		t.Error("NewApp() returned nil")
	}
	if app.stdout != stdout {
		t.Error("NewApp() did not assign stdout correctly")
	}
	if app.stderr != stderr {
		t.Error("NewApp() did not assign stderr correctly")
	}
}

// TestRunCauseValues verifies RunCause enum values are defined and distinct.
func TestRunCauseValues(t *testing.T) {
	if RunUnknownCommand == RunStorage {
		t.Error("RunUnknownCommand and RunStorage should have different values")
	}
}
