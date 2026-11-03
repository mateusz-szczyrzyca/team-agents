package main

import (
	"errors"
	"fmt"
	"testing"
)

// TestErrorClasses checks classification without adopting agentbus's numeric CLI exits.
func TestErrorClasses(t *testing.T) {
	// cases enumerate every shared class and its stable sentinel.
	cases := []struct {
		class    ErrorClass // class under test
		name     string     // diagnostic name
		sentinel error      // identity required by errors.Is
	}{
		{ClassInvalidArgument, "InvalidArgument", ErrInvalidArgument},
		{ClassNotFound, "NotFound", ErrNotFound},
		{ClassNotInTeam, "NotInTeam", ErrNotInTeam},
		{ClassPermission, "Permission", ErrPermission},
		{ClassConflict, "Conflict", ErrConflict},
		{ClassInvalidState, "InvalidState", ErrInvalidState},
		{ClassStorage, "Storage", ErrStorage},
		{ClassUnknownTool, "UnknownTool", ErrUnknownTool},
		{ClassProcess, "Process", ErrProcess},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.class.String() != tc.name || !errors.Is(tc.class.Sentinel(), tc.sentinel) || tc.class.ExitCode() != 1 {
				t.Fatalf("class %d: name=%q sentinel=%v exit=%d", tc.class, tc.class.String(), tc.class.Sentinel(), tc.class.ExitCode())
			}
		})
	}
}

// TestErrorClassUnknown checks safe diagnostics and failure exits for invalid class values.
func TestErrorClassUnknown(t *testing.T) {
	for _, value := range []ErrorClass{-1, 99} {
		if value.String() != fmt.Sprintf("ErrorClass(%d)", value) || value.ExitCode() != 1 || value.Sentinel() == nil {
			t.Fatalf("invalid class %d lacks safe fallback", value)
		}
	}
}

// TestCommandCauses checks the generated operation identity and exact causes.
func TestCommandCauses(t *testing.T) {
	// cases are the only failures currently exposed by local command dispatch.
	cases := []struct {
		cause CommandCause // exact generated cause
		name  string       // public cause name
	}{
		{CommandUnknownCommand, "UnknownCommand"},
		{CommandInvalidArgument, "InvalidArgument"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.cause.op() != "command" || tc.cause.String() != tc.name || tc.cause.info().class != ClassInvalidArgument {
				t.Fatalf("wrong generated cause metadata: %+v", tc.cause.info())
			}
			// failure must preserve the exact cause and support sentinel matching.
			failure := opErr(tc.cause, "bad argument %d", 7)
			if failure == nil {
				t.Fatal("constructor returned nil")
			}
			if failure.Cause != tc.cause || failure.Class() != ClassInvalidArgument || failure.CauseName() != tc.name {
				t.Fatalf("wrong error classification: %+v", failure)
			}
			if !errors.Is(failure.asError(), ErrInvalidArgument) || errors.Is(failure.asError(), ErrStorage) {
				t.Fatal("constructor lost sentinel identity")
			}
			// extracted proves errors.As preserves the concrete operation type through wrapping.
			var extracted *CommandError
			if !errors.As(fmt.Errorf("dispatch: %w", failure.asError()), &extracted) || extracted != failure {
				t.Fatal("errors.As lost operation error")
			}
		})
	}
}

// TestOperationErrorConversion checks typed nil and non-nil conversions at interface boundaries.
func TestOperationErrorConversion(t *testing.T) {
	// absent represents a successful typed operation result.
	var absent *CommandError
	if absent.asError() != nil {
		t.Fatal("typed nil became a non-nil error interface")
	}
	// underlying is retained alongside the class sentinel by the error's unwrap contract.
	underlying := errors.New("underlying failure")
	// failure represents an operation that wrapped a lower-level failure.
	failure := &CommandError{Cause: CommandInvalidArgument, Message: "invalid", Err: errors.Join(ErrInvalidArgument, underlying)}
	if failure.asError() == nil || !errors.Is(failure.asError(), underlying) || !errors.Is(failure.asError(), ErrInvalidArgument) {
		t.Fatal("conversion or unwrapping lost an error")
	}
	// wrapped models callers adding context without losing classification.
	wrapped := fmt.Errorf("outer: %w", failure.asError())
	if class, cause := classOf(wrapped); class != "InvalidArgument" || cause != "InvalidArgument" {
		t.Fatalf("classification lost through wrapping: %q %q", class, cause)
	}
	if class, cause := classOf(underlying); class != "" || cause != "" {
		t.Fatalf("untyped error gained classification: %q %q", class, cause)
	}
	if class, cause := classOf(nil); class != "" || cause != "" {
		t.Fatalf("nil error gained classification: %q %q", class, cause)
	}
}
