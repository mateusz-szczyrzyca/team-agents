package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// remoteWakeFixture creates a real hub-owned team and a node report without spawning any agent.
func remoteWakeFixture(t *testing.T) (*Bus, *Server, HubInstanceLease) {
	t.Helper()
	b := newBus(t)
	createTeam(t, b, "Remote", map[string][3]string{"lead@host": {"normal", "a", "active"}, "member@mac": {"normal", "b", "sleeping"}})
	mustWith(t, b, func(st *State) {
		team := st.Teams["Remote"]
		team.Lead = "lead@host"
		team.Members["member@mac"].Task = "task-member"
	})
	request := instanceRegisterFixture("mac", "session")
	request.Manifest.CurrentState.Tasks = map[string]HubTaskSummary{"task-member": {TaskID: "task-member", Agent: "member", Team: "Remote", Status: "sleeping"}}
	lease, err := (&HubInstanceRegistry{Bus: b}).Register("mac", "token:mac", request)
	if err != nil {
		t.Fatal(err)
	}
	return b, server(b, "lead@host", inTeam("Remote")), lease
}

// TestRemoteMessageWakeReachesNodePoll connects the normal send_message side effect to outbound command delivery.
func TestRemoteMessageWakeReachesNodePoll(t *testing.T) {
	b, lead, lease := remoteWakeFixture(t)
	local := captureWakes(t)
	chk(t).ok(toolResult(lead.send("member@mac", "continue backend work")))
	if len(*local) != 0 {
		t.Fatal("remote wake invoked the hub's local agent command")
	}
	hub := &Hub{Bus: b, Instances: &HubInstanceRegistry{Bus: b}}
	commands, err := hub.pollCommands(context.Background(), "mac", "token:mac", lease.Token, lease.Generation, 0)
	if err != nil || len(commands) != 1 {
		t.Fatalf("message did not reach node command poll: %v %v", commands, err)
	}
	command := commands[0]
	if command.Kind != CommandWake || command.Task != "task-member" || command.Params["member"] != "member@mac" {
		t.Fatalf("wake routed to the wrong task: %+v", command)
	}
	if memberOf(t, b, "Remote", "member@mac").State != MemberWaking {
		t.Fatal("queued member did not become waking")
	}
	// Before a node acknowledges, another source of work must not enqueue a duplicate wake.
	mustWith(t, b, func(st *State) { st.Teams["Remote"].Members["member@mac"].State = MemberSleeping })
	chk(t).ok(toolResult(lead.boardAddFor("Build backend", "run checks", "normal", "member@mac")))
	pending, serr := b.pendingCommands("mac", 0)
	if serr != nil || len(pending) != 1 {
		t.Fatalf("board duplicated a pending wake: %v %v", pending, serr)
	}
}

// TestRemoteWakeFailureKeepsUnreadWorkSleeping prevents an outage or journal failure from losing wake eligibility.
func TestRemoteWakeFailureKeepsUnreadWorkSleeping(t *testing.T) {
	for _, kind := range []string{"expired", "invalid journal", "unknown task"} {
		t.Run(kind, func(t *testing.T) {
			b, lead, _ := remoteWakeFixture(t)
			logs := captureLogs(t)
			local := captureWakes(t)
			registry := &HubInstanceRegistry{Bus: b}
			if kind == "invalid journal" {
				if err := os.WriteFile(filepath.Join(b.dir, commandStoreFile), []byte("invalid"), 0o600); err != nil {
					t.Fatal(err)
				}
			} else if serr := registry.withState(func(state *hubInstanceState) *StorageError {
				if kind == "expired" {
					state.Instances["mac"].Lease.ExpiresAt = Timestamp{Time: time.Now().Add(-time.Minute)}
				} else {
					state.Instances["mac"].Manifest.CurrentState.Tasks = nil
				}
				return registry.save(state)
			}); serr != nil {
				t.Fatal(serr)
			}
			chk(t).ok(toolResult(lead.send("member@mac", "retained during outage")))
			if memberOf(t, b, "Remote", "member@mac").State != MemberSleeping || len(*local) != 0 {
				t.Fatal("failed remote wake lost eligibility or spawned a local agent")
			}
			chk(t).ok(toolResult(lead.boardAddFor("Retained board work", "retry after return", "normal", "member@mac")))
			if member := memberOf(t, b, "Remote", "member@mac"); member.State != MemberSleeping || member.BoardSeen != 0 {
				t.Fatalf("failed board wake marked its work seen: %+v", member)
			}
			if !strings.Contains(logs.String(), `"effect":"queue remote member wake"`) {
				t.Fatal("failed wake side effect was not visible in logs")
			}
			messages, serr := b.messages()
			if serr != nil || len(messages) < 2 || messages[len(messages)-2].Text != "retained during outage" {
				t.Fatalf("wake failure lost its triggering message: %v %v", messages, serr)
			}
		})
	}
}

// TestRemoteWakeHasExactEnqueueCauses covers validation, reachability, capacity and storage failure causes.
func TestRemoteWakeHasExactEnqueueCauses(t *testing.T) {
	b, _, _ := remoteWakeFixture(t)
	mustWith(t, b, func(*State) {
		expectCause(t, b.queueMemberWakeLocked("Remote", "member", "../mac", "task-member"), CommandEnqueueInvalidParams)
		expectCause(t, b.queueMemberWakeLocked("Remote", "member", "gone", "task-member"), CommandEnqueueInstanceUnreachable)
		expectCause(t, b.queueMemberWakeLocked("Remote", "other", "mac", "task-member"), CommandEnqueueInvalidParams)
		store := &commandStore{Version: hubProtocolVersion, Queues: map[string]*commandQueue{}}
		queue := store.queue("mac")
		for i := 0; i < commandQueueLimit; i++ {
			queue.Pending = append(queue.Pending, Command{Seq: int64(i + 1), Kind: CommandStopTask, Task: "other", Params: map[string]string{"reason": "test"}})
		}
		if serr := b.saveCommandStore(store); serr != nil {
			t.Fatal(serr)
		}
		expectCause(t, b.queueMemberWakeLocked("Remote", "member", "mac", "task-member"), CommandEnqueueQueueFull)
	})
}
