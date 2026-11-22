package main

// Tests of the delegating line of the initialize instructions (written first): a run that may ask a worker — an
// investigator answering an investigation request among them, which reads no role text of a team — is told when to
// give a step to a worker and when not to; a worker, which may not ask, is not.

import (
	"strings"
	"testing"
)

func TestInstructionsTellARunThatMayDelegateWhenTo(t *testing.T) {
	b := newBus(t)
	for name, s := range map[string]*Server{
		"agent":        server(b, "a1"),
		"investigator": server(b, "i1", func(s *Server) { s.role = RoleInvestigator }),
	} {
		text, _ := initialized(t, s)
		for _, want := range []string{"delegate", "one result, named files and a check a machine runs",
			"never a decision, a design or a step whose check is reviewer"} {
			if !strings.Contains(text, want) {
				t.Errorf("%s: the instructions do not say %q: %q", name, want, text)
			}
		}
	}
}

func TestInstructionsTellAWorkerNothingOfDelegating(t *testing.T) {
	text, _ := initialized(t, server(newBus(t), "w1", asWorker))

	if strings.Contains(text, "delegate") {
		t.Fatalf("a worker may not delegate and is not told to: %q", text)
	}
}
