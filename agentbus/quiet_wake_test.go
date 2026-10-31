package main

import (
	"strings"
	"testing"
)

// A message wakes a sleeping member, and the log says which (member_woken_by); a quiet one (send_message wake=false:
// progress, a note that needs no reaction) wakes nobody — neither when it is sent nor when its reader goes to sleep
// with it unread. It is read on the member's next run.
func TestAQuietMessageWakesNobody(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "w": {"normal", "b", "sleeping"}})
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.sendMessage("w", "B4: the gate is at step 2 of 6", false)))
	if len(*woken) != 0 {
		t.Fatalf("a quiet message woke %v", *woken)
	}
	mustWith(t, b, func(st *State) {
		st.Teams["T"].Members["w"].State = MemberActive
	})
	mustWith(t, b, func(st *State) {
		if got, err := setMemberState(b, st, "T", "w", MemberSleeping); err != nil || got == "waking" {
			t.Errorf("going to sleep with only a quiet message unread: %q, %v", got, err)
		}
	})
	if len(*woken) != 0 {
		t.Fatalf("an unread quiet message woke %v at its reader's sleep", *woken)
	}
	chk(t).ok(toolResult(lead.send("w", "please take B5")))
	if len(*woken) != 1 || (*woken)[0] != "task-w" {
		t.Fatalf("a message should wake the member, woken %v", *woken)
	}
	if out := chk(t).ok(toolResult(server(b, "w", inTeam("T")).read(0))); !strings.Contains(out, "step 2 of 6") {
		t.Errorf("the quiet message is read on the next run, got:\n%s", out)
	}
}
