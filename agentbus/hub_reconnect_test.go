package main

import (
	"testing"
	"time"
)

// reapMac moves the clock past the lease of instance mac and reclaims its work.
func reapMac(t *testing.T, b *Bus, reg *HubInstanceRegistry, clock *time.Time) {
	t.Helper()
	*clock = clock.Add(181 * time.Second)
	if serr := reapUnreachable(b, reg); serr != nil {
		t.Fatal(serr)
	}
}

// reconnectMac registers instance mac again with a new session, as a restarted daemon does.
func reconnectMac(t *testing.T, reg *HubInstanceRegistry) {
	t.Helper()
	if _, err := reg.Register("mac", "node-mac", instanceRegisterFixture("mac", "session-mac-2")); err != nil {
		t.Fatal(err)
	}
}

// TestReconnectRestoresReclaimedMemberState verifies that a member reclaimed with its instance becomes sleeping again
// when the instance reconnects, so that it can be woken, and that its board item stays returned to the board.
func TestReconnectRestoresReclaimedMemberState(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	reapMac(t, b, reg, clock)
	reconnectMac(t, reg)
	team := teamState(t, b)
	m := team.Members["codex-1@mac"]
	if m.State != MemberSleeping || m.UnreachableFrom != "" {
		t.Fatalf("codex-1@mac is %s (from %q) after reconnect, want sleeping", m.State, m.UnreachableFrom)
	}
	if got := team.Members["claude-1"].State; got != MemberActive {
		t.Fatalf("a local member changed to %s", got)
	}
	if got := team.Board["B1"].Status; got != BoardTodo {
		t.Fatalf("reconnect gave B1 back to %s; it must stay on the board", got)
	}
}

// TestReconnectKeepsPausedMemberPaused verifies that a paused member keeps its pause, because its resume timer
// still applies.
func TestReconnectKeepsPausedMemberPaused(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	mustWith(t, b, func(st *State) { st.Teams["T"].Members["codex-1@mac"].State = MemberPaused })
	reapMac(t, b, reg, clock)
	reconnectMac(t, reg)
	if got := teamState(t, b).Members["codex-1@mac"].State; got != MemberPaused {
		t.Fatalf("codex-1@mac is %s after reconnect, want paused", got)
	}
}

// TestReconnectWithoutRecordedStateRestoresSleeping covers unreachable members written before UnreachableFrom
// existed: their previous state is unknown, so they come back sleeping.
func TestReconnectWithoutRecordedStateRestoresSleeping(t *testing.T) {
	b, reg, clock := reaperFixture(t)
	reaperTeam(t, b)
	reapMac(t, b, reg, clock)
	mustWith(t, b, func(st *State) { st.Teams["T"].Members["codex-1@mac"].UnreachableFrom = "" })
	reconnectMac(t, reg)
	if got := teamState(t, b).Members["codex-1@mac"].State; got != MemberSleeping {
		t.Fatalf("codex-1@mac is %s after reconnect, want sleeping", got)
	}
}

// TestHeartbeatRestoresUnreachableMember covers the heartbeat path: a member marked unreachable while the lease is
// still valid is restored by the next live report.
func TestHeartbeatRestoresUnreachableMember(t *testing.T) {
	b, reg, _ := reaperFixture(t)
	reaperTeam(t, b)
	mustWith(t, b, func(st *State) { st.Teams["T"].Members["codex-1@mac"].State = MemberUnreachable })
	fixture := instanceRegisterFixture("mac", "session-mac")
	lease, err := reg.Register("mac", "node-mac", fixture)
	if err != nil {
		t.Fatal(err)
	}
	req := HubInstanceHeartbeatRequest{Version: hubProtocolVersion, Instance: "mac", Sequence: fixture.Sequence + 1,
		Generation: lease.Generation, Token: lease.Token, Manifest: fixture.Manifest}
	if _, herr := reg.Heartbeat("mac", "node-mac", req); herr != nil {
		t.Fatal(herr)
	}
	if got := teamState(t, b).Members["codex-1@mac"].State; got != MemberSleeping {
		t.Fatalf("codex-1@mac is %s after heartbeat, want sleeping", got)
	}
}
