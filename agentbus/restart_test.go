package main

// Tests of a node's restart, on the lead (written first, docs/tdd.md): a remote member whose node is away longer
// than the grace is put out of its team (once); one back from a restart within the grace is told to finish its
// items and woken; one back after being put out rejoins as available, told so, and woken.

import (
	"strings"
	"testing"
	"time"
)

// lastSeen is the lead's last heartbeat of a node.
func lastSeen(t *testing.T, a *leadAPI, node string) time.Time {
	t.Helper()
	reg, err := a.bus.loadJobs()
	if err != nil {
		t.Fatal(err)
	}
	return reg.nodes[node].LastSeen.Time
}

// inbox is what a member has to read.
func inbox(t *testing.T, b *Bus, member string) string {
	t.Helper()
	return chk(t).ok(toolResult(server(b, member, inTeam("T")).read(0)))
}

func TestAbsentRemoteMemberIsPutOutAfterTheGrace(t *testing.T) {
	_, a, _, _ := remoteTeam(t)
	events := captureRemoteMember(t)
	seen := lastSeen(t, a, "node2")
	for _, after := range []time.Duration{5 * time.Minute, 11 * time.Minute, 12 * time.Minute} {
		if err := a.checkAbsentMembers(seen.Add(after), 10*time.Minute); err != nil {
			t.Fatal(err)
		}
	}
	if len(*events) != 1 || !strings.Contains((*events)[0], " out") {
		t.Errorf("events %q, want one: out (after the grace, once)", *events)
	}
}

func TestRemoteMemberBackFromARestartFinishesItsItems(t *testing.T) {
	b, a, _, _ := remoteTeam(t)
	for i := 0; i < 2; i++ { // reported twice before it runs: told once
		wakes, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "interrupted"}})
		if err != nil || len(wakes) != 1 || wakes[0] != "job-m" {
			t.Fatalf("heartbeat %d: wakes %v, %v; want job-m", i, wakes, err)
		}
	}
	got := inbox(t, b, "solo@node2")
	if !strings.Contains(got, "restarted") || !strings.Contains(got, "still yours") || strings.Count(got, "restarted") != 1 {
		t.Errorf("the member was not told once to finish its items:\n%s", got)
	}
}

func TestRemoteMemberBackAfterBeingPutOutRejoinsAsAvailable(t *testing.T) {
	b, a, _, _ := remoteTeam(t)
	events := captureRemoteMember(t)
	if err := a.checkAbsentMembers(lastSeen(t, a, "node2").Add(11*time.Minute), 10*time.Minute); err != nil {
		t.Fatal(err)
	}
	// the script puts it out (team_after_run → member_out): done in its team
	if err := transact(b, CmdTeamSetStorage, func(st *State) *CmdTeamSetError {
		_, err := setMemberState(b, st, "T", "solo@node2", MemberDone)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	wakes, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "interrupted"}})
	if err != nil || len(wakes) != 1 {
		t.Fatalf("wakes %v, %v; want job-m", wakes, err)
	}
	if st, _ := memberState(t, b, "solo@node2"); st != MemberActive {
		t.Errorf("the member back is %s, want active", st)
	}
	if got := inbox(t, b, "solo@node2"); !strings.Contains(got, "out of the team") {
		t.Errorf("the member back was not told it had been out:\n%s", got)
	}
	if !strings.Contains(strings.Join(*events, "|"), " running") {
		t.Errorf("its proxy task was not set running again: %q", *events)
	}
}
