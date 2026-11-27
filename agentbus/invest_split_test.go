package main

import (
	_ "embed"
	"encoding/json"
	"errors"
	"testing"
)

// stateBeforeSplitKind preserves question and review requests written before split requests.
//
//go:embed testdata/state-before-split-kind.json
var stateBeforeSplitKind []byte

func TestOldStateWithoutSplitKindLoads(t *testing.T) {
	b := installState(t, stateBeforeSplitKind)
	for range 2 {
		mustWith(t, b, func(st *State) {
			if len(st.Investigations) != 2 || st.Investigations["1"].Kind != KindInvestigation || st.Investigations["2"].Kind != KindReview {
				t.Fatalf("legacy requests changed: %+v", st.Investigations)
			}
		})
	}
}

// splitRequester builds a team with a designated lead for permission checks.
func splitRequester(t *testing.T, caller string) *Server {
	t.Helper()
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{
		"lead":   {"normal", "provider", "active"},
		"member": {"normal", "provider", "active"},
	})
	mustWith(t, b, func(st *State) { st.Teams["T"].Lead = "lead" })
	return server(b, caller, inTeam("T"))
}

func TestSplitRequestRefusedForAMember(t *testing.T) {
	for _, kind := range []string{"split", "conflict"} {
		s := splitRequester(t, "member")
		_, err := s.callTool("request_investigation", map[string]any{"question": "Help", "kind": kind, "item": "1.1"})
		var denied *RequestInvestigationError // dispatcher preserves the operation's typed cause
		if !errors.As(err, &denied) || denied.Cause != RequestInvestigationNotLead {
			t.Fatalf("%s: got %v, want RequestInvestigationNotLead", kind, err)
		}
	}
}

func TestSplitRequestNeedsItem(t *testing.T) {
	for _, kind := range []string{"split", "conflict"} {
		s := splitRequester(t, "lead")
		_, err := s.callTool("request_investigation", map[string]any{"question": "Help", "kind": kind})
		var denied *RequestInvestigationError // missing item is an argument error
		if !errors.As(err, &denied) || denied.Cause != RequestInvestigationMissingItem {
			t.Fatalf("%s: got %v, want RequestInvestigationMissingItem", kind, err)
		}
	}
}

func TestSplitRequestPreservesKindAndItem(t *testing.T) {
	for _, kind := range []string{"split", "conflict", "question", ""} {
		s := splitRequester(t, "lead")
		if _, err := s.callTool("request_investigation", map[string]any{"question": "Découper", "kind": kind, "item": "1.1"}); err != nil {
			t.Fatal(err)
		}
		mustWith(t, s.bus, func(st *State) {
			inv := st.Investigations["1"]
			want := kind
			if want == "question" {
				want = ""
			}
			if inv == nil || string(inv.Kind) != want {
				t.Fatalf("kind %q: got %+v", kind, inv)
			}
			data, err := json.Marshal(inv)
			if err != nil {
				t.Fatal(err)
			}
			var fields map[string]any // assert the persisted interface rather than private implementation
			if err := json.Unmarshal(data, &fields); err != nil {
				t.Fatal(err)
			}
			if fields["item"] != "1.1" {
				t.Fatalf("missing item: %s", data)
			}
		})
	}
}

func TestRequestInvestigationRejectsUnknownKind(t *testing.T) {
	s := splitRequester(t, "lead")
	_, err := s.callTool("request_investigation", map[string]any{"question": "Help", "kind": "unknown"})
	var denied *RequestInvestigationError // invalid enum retains the operation cause
	if !errors.As(err, &denied) || denied.Cause != RequestInvestigationInvalidKind {
		t.Fatalf("got %v, want RequestInvestigationInvalidKind", err)
	}
}

// requestInvestigation keeps existing tests on the legacy question-call interface.
func (s *Server) requestInvestigation(question, context string, wait bool) (string, *RequestInvestigationError) {
	return s.requestInvestigationKind(question, context, wait, "", "")
}

func TestLegacyQuestionPreservesContext(t *testing.T) {
	s := splitRequester(t, "lead")
	if _, err := s.requestInvestigation("Investigate", "Contexte vérifié", false); err != nil {
		t.Fatal(err)
	}
	mustWith(t, s.bus, func(st *State) {
		inv := st.Investigations["1"]
		if inv == nil || inv.Kind != KindInvestigation || inv.Context != "Contexte vérifié" {
			t.Fatalf("legacy question lost context: %+v", inv)
		}
	})
}
