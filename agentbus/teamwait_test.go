package main

// Tests of what a team waits for — written first (docs/tdd.md). The stuck detection must not call a team stuck
// while a member's worker request or investigation is still running: its result wakes the member.

import (
	"reflect"
	"testing"
)

// Rule: a worker request is open until it is done or failed.
func TestHelperStatusOpen(t *testing.T) {
	for s, want := range map[HelperStatus]bool{
		HelperQueued: true, HelperStarting: true, HelperRunning: true, HelperStatus("done"): false, HelperStatus("failed"): false,
	} {
		if got := s.open(); got != want {
			t.Errorf("%s.open() = %v, want %v", s, got, want)
		}
	}
}

// Rule: an investigation is open until it is done, failed or rejected — waiting for the human's answers included.
func TestInvestigationStatusOpen(t *testing.T) {
	for s, want := range map[InvestigationStatus]bool{
		InvQueued: true, InvWaitroom: true, InvStarting: true, InvRunning: true, InvWaitingHuman: true,
		InvDone: false, InvFailed: false, InvRejected: false,
	} {
		if got := s.open(); got != want {
			t.Errorf("%s.open() = %v, want %v", s, got, want)
		}
	}
}

// Rule: only the members' own unfinished requests count — finished ones and other tasks' requests do not.
func TestTeamWaitingOn(t *testing.T) {
	st := &State{
		Teams: map[string]*Team{"T": {ID: "T", Members: map[string]*TeamMember{
			"lead": {Task: "task-lead"}, "member": {Task: "task-member"},
		}}},
		Helpers: map[string]*Helper{
			"1": {ID: 1, From: "lead", FromTask: "task-lead", Status: HelperRunning},
			"2": {ID: 2, From: "member", FromTask: "task-member", Status: HelperStatus("done")},
			"3": {ID: 3, From: "solo", FromTask: "task-solo", Status: HelperQueued},
		},
		Investigations: map[string]*Investigation{
			"7": {ID: 7, FromAgent: "member", FromTask: "task-member", Status: InvWaitingHuman},
			"8": {ID: 8, FromAgent: "lead", FromTask: "task-lead", Status: InvDone},
			"9": {ID: 9, FromAgent: "solo", FromTask: "task-solo", Status: InvQueued},
		},
	}
	want := []string{"#7 (investigation of member, waiting_human)", "H1 (worker request of lead, running)"}
	if got := teamWaitingOn(st, "T"); !reflect.DeepEqual(got, want) {
		t.Errorf("teamWaitingOn:\n got %q\nwant %q", got, want)
	}
	if got := teamWaitingOn(st, "no-such-team"); got != nil {
		t.Errorf("an unknown team waits for %q", got)
	}
	st.Helpers["1"].Status = HelperStatus("done")
	st.Investigations["7"].Status = InvDone
	if got := teamWaitingOn(st, "T"); got != nil {
		t.Errorf("all finished, still waiting for %q", got)
	}
}
