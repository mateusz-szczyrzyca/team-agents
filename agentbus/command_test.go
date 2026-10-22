package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestCommandLifecycle(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)
	cm := NewCommandManager(b, im)

	// Register "mac"
	m := sampleManifest("mac")
	regResp, regErr := im.Register("mac", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	if regErr != nil {
		t.Fatal(regErr)
	}

	// 1. Enqueue command for unreachable/unknown instance -> CommandEnqueueInstanceUnreachable
	_, err := cm.Enqueue(NodeCommand{
		Instance: "unknown-inst",
		Kind:     CmdStartRun,
	})
	expectCause(t, err, CommandEnqueueInstanceUnreachable)

	// 2. Enqueue command with invalid kind -> CommandEnqueueInvalidCommand
	_, err = cm.Enqueue(NodeCommand{
		Instance: "mac",
		Kind:     "bad_kind",
	})
	expectCause(t, err, CommandEnqueueInvalidCommand)

	// 3. Enqueue command successfully
	cmd, err := cm.Enqueue(NodeCommand{
		Instance:  "mac",
		Kind:      CmdStartRun,
		Task:      "task-123",
		Agent:     "codex-1",
		Run:       1,
		PromptRef: "prompts/protocol.md",
	})
	if err != nil {
		t.Fatalf("unexpected enqueue error: %v", err)
	}
	if cmd.ID == "" || cmd.Status != CmdPending {
		t.Fatalf("unexpected command state: %+v", cmd)
	}

	// 4. Duplicate command ID -> CommandEnqueueDuplicateCommand
	_, err = cm.Enqueue(NodeCommand{
		ID:       cmd.ID,
		Instance: "mac",
		Kind:     CmdStartRun,
	})
	expectCause(t, err, CommandEnqueueDuplicateCommand)

	// 5. Poll commands with instance mismatch -> CommandPollInstanceMismatch
	_, pollErr := cm.Poll(context.Background(), "other", CommandPollRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
	}, 0)
	expectCause(t, pollErr, CommandPollInstanceMismatch)

	// 6. Poll commands with stale lease -> CommandPollStaleLease
	_, pollErr = cm.Poll(context.Background(), "mac", CommandPollRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        999,
	}, 0)
	expectCause(t, pollErr, CommandPollStaleLease)

	// 7. Poll commands successfully returns the queued command
	pollResp, pollErr := cm.Poll(context.Background(), "mac", CommandPollRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
	}, 0)
	if pollErr != nil {
		t.Fatalf("unexpected poll error: %v", pollErr)
	}
	if len(pollResp.Commands) != 1 || pollResp.Commands[0].ID != cmd.ID {
		t.Fatalf("expected 1 command with id %s, got %+v", cmd.ID, pollResp.Commands)
	}

	// 8. Ack command with invalid result format -> CommandAckInvalidResult
	_, ackErr := cm.Ack("mac", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    cmd.ID,
		Result:       "invalid_status",
	})
	expectCause(t, ackErr, CommandAckInvalidResult)

	// 9. Ack command with wrong instance -> CommandAckInstanceMismatch
	_, ackErr = cm.Ack("other", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    cmd.ID,
		Result:       "started",
	})
	expectCause(t, ackErr, CommandAckInstanceMismatch)

	// 10. Ack command with stale lease -> CommandAckStaleLease
	_, ackErr = cm.Ack("mac", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        999,
		CommandID:    cmd.ID,
		Result:       "started",
	})
	expectCause(t, ackErr, CommandAckStaleLease)

	// 11. Ack non-existent command -> CommandAckCommandNotFound
	_, ackErr = cm.Ack("mac", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    "cmd-99999",
		Result:       "started",
	})
	expectCause(t, ackErr, CommandAckCommandNotFound)

	// 12. Successful Ack
	ackResp, ackErr := cm.Ack("mac", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    cmd.ID,
		Result:       "started",
	})
	if ackErr != nil {
		t.Fatalf("unexpected ack error: %v", ackErr)
	}
	if ackResp.CommandID != cmd.ID || ackResp.Status != string(CmdAcked) {
		t.Fatalf("unexpected ack response: %+v", ackResp)
	}

	// 13. Idempotent repeated Ack with same result succeeds
	ackRespRepeat, ackErrRepeat := cm.Ack("mac", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    cmd.ID,
		Result:       "started",
	})
	if ackErrRepeat != nil || ackRespRepeat.CommandID != cmd.ID {
		t.Fatalf("expected idempotent ack success, got resp=%+v err=%v", ackRespRepeat, ackErrRepeat)
	}

	// 14. Conflicting Ack with different result -> CommandAckConflictAck
	_, ackErrConflict := cm.Ack("mac", CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    cmd.ID,
		Result:       "stopped",
	})
	expectCause(t, ackErrConflict, CommandAckConflictAck)
}

func TestCommandLongPollWakeup(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)
	cm := NewCommandManager(b, im)

	// Register "mac"
	m := sampleManifest("mac")
	regResp, err := im.Register("mac", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	if err != nil {
		t.Fatal(err)
	}

	// Start long-poll in background
	pollDone := make(chan *CommandPollResponse, 1)
	go func() {
		resp, pErr := cm.Poll(context.Background(), "mac", CommandPollRequest{
			Version:      hubProtocolVersion,
			InstanceName: "mac",
			Lease:        regResp.Lease,
		}, 5*time.Second)
		if pErr != nil {
			t.Errorf("poll error: %v", pErr)
		}
		pollDone <- resp
	}()

	// Brief sleep to ensure long-poll is waiting
	time.Sleep(50 * time.Millisecond)

	// Enqueue command: should wake up poll immediately
	cmd, eErr := cm.Enqueue(NodeCommand{
		Instance: "mac",
		Kind:     CmdWake,
		Agent:    "codex-1",
		Reason:   "new board item",
	})
	if eErr != nil {
		t.Fatal(eErr)
	}

	select {
	case resp := <-pollDone:
		if len(resp.Commands) != 1 || resp.Commands[0].ID != cmd.ID {
			t.Fatalf("expected woke poll to return command %s, got %+v", cmd.ID, resp.Commands)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("long poll was not woken within 2 seconds")
	}
}

func TestCommandHTTPRoutes(t *testing.T) {
	b := newBus(t)
	h := &Hub{
		Bus: b,
		Authenticate: func(r *http.Request) (string, *HubAuthError) {
			token := r.Header.Get("Authorization")
			if token == "Bearer mac-token" {
				return "mac", nil
			}
			return "", opErr(HubAuthWrongToken, "invalid token")
		},
		PollDuration: 50 * time.Millisecond,
	}

	// Register instance first
	m := sampleManifest("mac")
	regBody, _ := json.Marshal(InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	wReg := httptest.NewRecorder()
	reqReg := httptest.NewRequest(http.MethodPost, "/v1/instances/register", bytes.NewReader(regBody))
	reqReg.Header.Set("Authorization", "Bearer mac-token")
	h.ServeHTTP(wReg, reqReg)
	if wReg.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", wReg.Code, wReg.Body.String())
	}
	var regResp InstanceRegisterResponse
	if err := json.Unmarshal(wReg.Body.Bytes(), &regResp); err != nil {
		t.Fatal(err)
	}

	// 1. Poll with no commands queued -> 204 No Content
	pollBody, _ := json.Marshal(CommandPollRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
	})
	wPoll := httptest.NewRecorder()
	reqPoll := httptest.NewRequest(http.MethodPost, "/v1/commands/poll", bytes.NewReader(pollBody))
	reqPoll.Header.Set("Authorization", "Bearer mac-token")
	h.ServeHTTP(wPoll, reqPoll)
	if wPoll.Code != http.StatusNoContent {
		t.Fatalf("expected 204 No Content, got %d: %s", wPoll.Code, wPoll.Body.String())
	}

	// 2. Enqueue command via manager
	cmd, eErr := h.Commands.Enqueue(NodeCommand{
		Instance: "mac",
		Kind:     CmdStopTask,
		Task:     "task-456",
		Reason:   "user cancellation",
	})
	if eErr != nil {
		t.Fatal(eErr)
	}

	// 3. Poll with queued command -> 200 OK with command
	wPoll2 := httptest.NewRecorder()
	reqPoll2 := httptest.NewRequest(http.MethodPost, "/v1/commands/poll", bytes.NewReader(pollBody))
	reqPoll2.Header.Set("Authorization", "Bearer mac-token")
	h.ServeHTTP(wPoll2, reqPoll2)
	if wPoll2.Code != http.StatusOK {
		t.Fatalf("expected 200 OK, got %d: %s", wPoll2.Code, wPoll2.Body.String())
	}
	var pollResp CommandPollResponse
	if err := json.Unmarshal(wPoll2.Body.Bytes(), &pollResp); err != nil {
		t.Fatal(err)
	}
	if len(pollResp.Commands) != 1 || pollResp.Commands[0].ID != cmd.ID {
		t.Fatalf("expected command %s, got %+v", cmd.ID, pollResp.Commands)
	}

	// 4. Ack command via HTTP -> 200 OK
	ackBody, _ := json.Marshal(CommandAckRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		CommandID:    cmd.ID,
		Result:       "stopped",
	})
	wAck := httptest.NewRecorder()
	reqAck := httptest.NewRequest(http.MethodPost, "/v1/commands/ack", bytes.NewReader(ackBody))
	reqAck.Header.Set("Authorization", "Bearer mac-token")
	h.ServeHTTP(wAck, reqAck)
	if wAck.Code != http.StatusOK {
		t.Fatalf("expected 200 OK for ack, got %d: %s", wAck.Code, wAck.Body.String())
	}
}
