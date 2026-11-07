package main

import (
	"strings"
	"testing"
)

// TestClaimLeadTakesOverFromALeadThatIsOut: a coordinator out of its team — finished on a failure (done), stopped or
// excluded — is replaced by the normal member that claims the role, as a hung one is; told "the coordinator is …"
// instead, its members waited for a coordinator that would never come back.
func TestClaimLeadTakesOverFromALeadThatIsOut(t *testing.T) {
	for _, out := range []MemberState{MemberDone, MemberStopped, MemberExcluded} {
		t.Run(string(out), func(t *testing.T) {
			b := newBus(t)
			createTeam(t, b, "T", map[string][3]string{
				"n1": {"normal", "anthropic", "active"}, "n2": {"normal", "openai", "sleeping"},
			})
			n1 := server(b, "n1", inTeam("T"))
			chk(t).ok(toolResult(n1.claimLead()))
			mustWith(t, b, func(st *State) { st.Teams["T"].Members["n1"].State = out })
			n2 := server(b, "n2", inTeam("T"))
			if got := chk(t).ok(toolResult(n2.claimLead())); !strings.Contains(got, "You are the coordinator") {
				t.Fatalf("n2 claiming after n1 is %s: %q", out, got)
			}
			mustWith(t, b, func(st *State) {
				if lead := st.Teams["T"].Lead; lead != "n2" {
					t.Fatalf("lead = %q, want n2", lead)
				}
			})
		})
	}
}
