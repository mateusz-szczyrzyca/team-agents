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

// TestReconnectWakesMemberWithUnreadMessage verifies that a message sent while the member's instance was away
// wakes the member through the node queue as soon as the instance reconnects, instead of waiting for the next message.
func TestReconnectWakesMemberWithUnreadMessage(t *testing.T) {
	b := newBus(t)
	clock := time.Now().UTC() // the queued wake checks the lease against the real clock
	reg := &HubInstanceRegistry{Bus: b, TTL: 180 * time.Second, Now: func() time.Time { return clock }}
	createTeam(t, b, "Remote", map[string][3]string{"lead@host": {"normal", "a", "active"}, "member@mac": {"normal", "b", "active"}})
	mustWith(t, b, func(st *State) {
		team := st.Teams["Remote"]
		team.Lead = "lead@host"
		team.Members["member@mac"].Task = "task-member"
	})
	if _, err := reg.Register("mac", "node-mac", reportingRegistration("session-mac")); err != nil {
		t.Fatal(err)
	}
	reapMac(t, b, reg, &clock)
	lead := server(b, "lead@host", inTeam("Remote"))
	chk(t).ok(toolResult(lead.send("member@mac", "continue backend work")))
	if pending, serr := b.pendingCommands("mac", 0); serr != nil || len(pending) != 0 {
		t.Fatalf("a message to an unreachable member queued a wake: %v %v", pending, serr)
	}
	reconnectWith(t, reg, reportingRegistration("session-mac-2"))
	if got := memberOf(t, b, "Remote", "member@mac").State; got != MemberWaking {
		t.Fatalf("member@mac is %s after reconnect, want waking", got)
	}
	pending, serr := b.pendingCommands("mac", 0)
	if serr != nil || len(pending) != 1 || pending[0].Kind != CommandWake || pending[0].Params["member"] != "member@mac" {
		t.Fatalf("reconnect did not queue exactly one wake for member@mac: %v %v", pending, serr)
	}
}

// reportingRegistration registers instance mac and reports the task of member@mac in team Remote.
func reportingRegistration(session string) HubInstanceRegisterRequest {
	request := instanceRegisterFixture("mac", session)
	request.Manifest.CurrentState.Tasks = map[string]HubTaskSummary{"task-member": {TaskID: "task-member", Agent: "member", Team: "Remote", Status: "sleeping"}}
	return request
}

// reconnectWith registers instance mac with the given request, as a restarted daemon does.
func reconnectWith(t *testing.T, reg *HubInstanceRegistry, request HubInstanceRegisterRequest) {
	t.Helper()
	if _, err := reg.Register("mac", "node-mac", request); err != nil {
		t.Fatal(err)
	}
}
