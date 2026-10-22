package main

import (
	"errors"
	"testing"
)

// TestUnavailableClass checks the class added for the hub: its name, its CLI exit status (11, after Process 10)
// and that its sentinel is distinct from every other class.
func TestUnavailableClass(t *testing.T) {
	if ClassUnavailable.String() != "Unavailable" || ClassUnavailable.ExitCode() != 11 {
		t.Fatalf("Unavailable: %q/%d, want Unavailable/11", ClassUnavailable.String(), ClassUnavailable.ExitCode())
	}
	for _, other := range []ErrorClass{ClassInvalidArgument, ClassNotFound, ClassNotInTeam, ClassPermission,
		ClassConflict, ClassInvalidState, ClassStorage, ClassUnknownTool, ClassProcess} {
		if errors.Is(ErrUnavailable, other.Sentinel()) {
			t.Errorf("ErrUnavailable is the sentinel of %v", other)
		}
	}
	if !errors.Is(ClassUnavailable.Sentinel(), ErrUnavailable) {
		t.Error("ClassUnavailable.Sentinel() is not ErrUnavailable")
	}
}

// TestCauseByName checks the generated lookup that rebuilds a cause received on the wire from its name:
// every cause of an operation is found by its name, and unknown names are not mapped to a known cause.
func TestCauseByName(t *testing.T) {
	c, ok := QuizStartCauseByName("NotCoordinator")
	if !ok || c != QuizStartNotCoordinator {
		t.Fatalf("NotCoordinator: got %v/%v, want QuizStartNotCoordinator/true", c, ok)
	}
	if c, ok := QuizStartCauseByName("Storage"); !ok || c.String() != "Storage" {
		t.Errorf("Storage: got %v/%v, want the generated Storage cause", c, ok)
	}
	if _, ok := QuizStartCauseByName("NoSuchCause"); ok {
		t.Error("an unknown name was mapped to a cause")
	}
	if _, ok := QuizStartCauseByName(""); ok {
		t.Error("the empty name was mapped to a cause")
	}
}

// TestForwardedOperationsHaveOutcomeUnknown checks that an operation the hub forwards gets the OutcomeUnknown cause
// (class InvalidState) and that an operation the hub does not forward does not.
func TestForwardedOperationsHaveOutcomeUnknown(t *testing.T) {
	c, ok := QuizStartCauseByName("OutcomeUnknown")
	if !ok || c.op() != "quiz_start" || c.info().class != ClassInvalidState {
		t.Fatalf("quiz_start OutcomeUnknown: got %v/%v, want the InvalidState cause of quiz_start", c, ok)
	}
	if _, ok := ToolCallCauseByName("OutcomeUnknown"); ok {
		t.Error("tools/call is not forwarded but has an OutcomeUnknown cause")
	}
}

// TestHubCauses checks the causes of the hub route and of the hub subcommand that the wire and the CLI rely on.
func TestHubCauses(t *testing.T) {
	for _, name := range []string{"Malformed", "UnknownOperation", "Identity", "RequestConflict", "Expired", "Overloaded", "OutcomeUnknown", "Storage"} {
		if _, ok := HubCallCauseByName(name); !ok {
			t.Errorf("HubCall has no %s cause", name)
		}
	}
	if c, ok := HubCallCauseByName("Overloaded"); !ok || c.info().class != ClassUnavailable {
		t.Errorf("Overloaded: got %v/%v, want the Unavailable class", c, ok)
	}
	for _, name := range []string{"InvalidConfig", "Listen", "Storage"} {
		if _, ok := HubServeCauseByName(name); !ok {
			t.Errorf("HubServe has no %s cause", name)
		}
	}
}

// TestOpErrorWireAccessors checks the accessors the hub uses to encode an error: the operation name and the
// message without the operation prefix.
func TestOpErrorWireAccessors(t *testing.T) {
	e := &OpError[QuizStartCause]{Cause: QuizStartNotCoordinator, Message: "only the coordinator can start a quiz"}
	if e.Operation() != "quiz_start" {
		t.Errorf("Operation() = %q, want quiz_start", e.Operation())
	}
	if e.ErrorMessage() != "only the coordinator can start a quiz" {
		t.Errorf("ErrorMessage() = %q", e.ErrorMessage())
	}
}

// TestHubAuthCauses checks the causes of the hub client authentication that the auth modes return.
func TestHubAuthCauses(t *testing.T) {
	for _, name := range []string{"MissingCredentials", "WrongToken", "UnknownNode", "AddressMismatch", "DuplicateInstanceName", "LocalAPIFailed", "InvalidMode"} {
		if _, ok := HubAuthCauseByName(name); !ok {
			t.Errorf("HubAuth has no %s cause", name)
		}
	}
	if c, ok := HubAuthCauseByName("LocalAPIFailed"); !ok || c.info().class != ClassUnavailable {
		t.Errorf("LocalAPIFailed: got %v/%v, want the Unavailable class", c, ok)
	}
}

// TestWireOperationError checks that a wire error (operation name, cause name, message) is rebuilt as the
// operation's own error type with the same cause, class and message, and that anything unknown is refused.
func TestWireOperationError(t *testing.T) {
	err, ok := wireOperationError("quiz_start", "NotCoordinator", "only the coordinator may start a quiz")
	if !ok {
		t.Fatal("quiz_start/NotCoordinator was not rebuilt")
	}
	var typed *QuizStartError
	if !errors.As(err, &typed) {
		t.Fatalf("got %T, want *QuizStartError", err)
	}
	if typed.Cause != QuizStartNotCoordinator || typed.Class() != ClassPermission || typed.ErrorMessage() != "only the coordinator may start a quiz" {
		t.Errorf("rebuilt %v/%v/%q, want NotCoordinator/Permission/message", typed.Cause, typed.Class(), typed.ErrorMessage())
	}
	outcome, ok := wireOperationError("quiz_start", "OutcomeUnknown", "x")
	if !ok || !errors.As(outcome, &typed) || typed.Class() != ClassInvalidState {
		t.Errorf("forwarded OutcomeUnknown: got %v/%v, want an InvalidState error", outcome, ok)
	}
	if _, ok := wireOperationError("quiz_start", "NoSuchCause", "x"); ok {
		t.Error("an unknown cause was rebuilt")
	}
	if _, ok := wireOperationError("no_such_operation", "Storage", "x"); ok {
		t.Error("an unknown operation was rebuilt")
	}
	if _, ok := wireOperationError("release", "Storage", "x"); ok {
		t.Error("release is shared by a tool and a CLI subcommand and must stay unresolved")
	}
}
