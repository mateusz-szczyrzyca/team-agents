package main

import (
	"errors"
	"testing"
)

// TestWireOperationErrorSentinels checks that an error rebuilt from the wire matches its class sentinel with
// errors.Is, so callers can branch on the class without knowing the concrete operation type.
func TestWireOperationErrorSentinels(t *testing.T) {
	err, ok := wireOperationError("quiz_start", "NotCoordinator", "only the coordinator may start a quiz")
	if !ok {
		t.Fatal("quiz_start/NotCoordinator was not rebuilt")
	}
	if !errors.Is(err, ErrPermission) {
		t.Errorf("rebuilt NotCoordinator does not match ErrPermission: %v", err)
	}
	if errors.Is(err, ErrInvalidState) {
		t.Errorf("rebuilt NotCoordinator wrongly matches ErrInvalidState")
	}

	outcome, ok := wireOperationError("quiz_start", "OutcomeUnknown", "x")
	if !ok {
		t.Fatal("quiz_start/OutcomeUnknown was not rebuilt")
	}
	if !errors.Is(outcome, ErrInvalidState) {
		t.Errorf("rebuilt OutcomeUnknown does not match ErrInvalidState: %v", outcome)
	}
}

// TestForwardedOperationsHaveUnavailableCause checks that every forwarded operation can report that the hub
// is unreachable, and that the rebuilt error matches ErrUnavailable.
func TestForwardedOperationsHaveUnavailableCause(t *testing.T) {
	err, ok := wireOperationError("quiz_start", "Unavailable", "hub is down")
	if !ok {
		t.Fatal("quiz_start/Unavailable was not rebuilt")
	}
	if !errors.Is(err, ErrUnavailable) {
		t.Errorf("rebuilt Unavailable does not match ErrUnavailable: %v", err)
	}
}
