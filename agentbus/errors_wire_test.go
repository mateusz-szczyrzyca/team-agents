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
	for _, name := range []string{"Malformed", "UnknownOperation", "Identity", "RequestConflict", "Expired", "Overloaded", "Storage"} {
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
