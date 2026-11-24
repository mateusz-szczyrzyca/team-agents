package main

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
)

func TestOwnedReceiptRecoverySkipsThePersistedPrefixAcrossLoads(t *testing.T) {
	b, cmd := ownedFixture(t)
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("start = %s, %v", got, err)
	}
	cmd.Action, cmd.Status, cmd.Notice = "settle", HelperDone, "served"
	if got, err := b.helperOwned(cmd); err != nil || got != ownedApplied {
		t.Fatalf("settle = %s, %v", got, err)
	}
	if _, err := b.load(); err != nil {
		t.Fatal(err)
	}
	// Poison the already consumed prefix without changing its byte length. Decoding it
	// again would fail: each restarted load must seek past it, even with new traffic.
	data, err := os.ReadFile(b.path("messages.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	poisoned := bytes.Replace(data, []byte(`"helper_result"`), []byte(`123456789012345`), 1)
	if len(poisoned) != len(data) || bytes.Equal(poisoned, data) {
		t.Fatal("receipt prefix not poisoned")
	}
	if err := os.WriteFile(b.path("messages.jsonl"), poisoned, 0o600); err != nil { // #nosec G703 -- the path is fixed in t.TempDir; only the intentional corrupt receipt is tainted.
		t.Fatal(err)
	}
	for i := 0; i < 4; i++ {
		restarted := &Bus{dir: b.dir}
		st, serr := restarted.load()
		if serr != nil {
			t.Fatalf("load %d decoded consumed history: %v", i, serr)
		}
		if st.Helpers["1"].Status != HelperDone {
			t.Fatal("settled request changed")
		}
		if _, serr := restarted.post(st, "human", "all", EventChat{Body: "unrelated traffic"}); serr != nil {
			t.Fatal(serr)
		}
		if serr := restarted.save(st); serr != nil {
			t.Fatal(serr)
		}
		if _, serr := restarted.load(); serr != nil {
			t.Fatal(serr)
		}
		state, err := os.ReadFile(b.path("state.json"))
		if err != nil {
			t.Fatal(err)
		}
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(state, &fields); err != nil {
			t.Fatal(err)
		}
		var offset int64
		if err := json.Unmarshal(fields["owned_receipt_offset"], &offset); err != nil {
			t.Fatal(err)
		}
		info, err := os.Stat(b.path("messages.jsonl"))
		if err != nil {
			t.Fatal(err)
		}
		if offset != info.Size() {
			t.Fatalf("receipt offset = %d, want %d", offset, info.Size())
		}
	}
}

func TestOwnedReceiptRecoveryRejectsOffsetsOutsideTheLog(t *testing.T) {
	for _, offset := range []int64{-1, 1} {
		b, cmd := ownedFixture(t)
		if _, err := b.helperOwned(cmd); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(b.path("messages.jsonl"), nil, 0o600); err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(b.path("state.json"))
		if err != nil {
			t.Fatal(err)
		}
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(data, &fields); err != nil {
			t.Fatal(err)
		}
		fields["owned_receipt_offset"], err = json.Marshal(offset)
		if err != nil {
			t.Fatal(err)
		}
		data, err = json.Marshal(fields)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(b.path("state.json"), data, 0o600); err != nil {
			t.Fatal(err)
		}
		if _, serr := b.load(); serr == nil {
			t.Fatalf("accepted receipt offset %d outside empty log", offset)
		}
	}
}

func TestOwnedReceiptRecoveryReportsANonterminalReceipt(t *testing.T) {
	b, cmd := ownedFixture(t)
	if _, err := b.helperOwned(cmd); err != nil {
		t.Fatal(err)
	}
	st, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	if _, serr := b.post(st, "agent", "lead", EventScript{Notice: NoticeHelperResult, Fields: map[string]string{
		"owned_request": "1", "run": cmd.Run, "worker": cmd.Worker, "task": cmd.Task,
		"status": string(HelperRunning),
	}}); serr != nil {
		t.Fatal(serr)
	}
	if _, serr := b.load(); serr == nil {
		t.Fatal("accepted a nonterminal final receipt")
	}
}

func TestOwnedRequestBeforeTheReceiptCursorStillSettlesOnce(t *testing.T) {
	b := newBus(t)
	data, err := os.ReadFile("testdata/state-owned-before-cursor.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(b.path("state.json"), data, 0o600); err != nil { // #nosec G703 -- the path is fixed in t.TempDir; data is the legacy JSON fixture, not a path.
		t.Fatal(err)
	}
	cmd := ownedCommand{ID: 1, Action: "settle", Worker: "worker", Task: "worker-task", Run: "run-1", Status: HelperDone, Notice: "served"}
	for _, want := range []ownedOutcome{ownedApplied, ownedReplay} {
		got, err := (&Bus{dir: b.dir}).helperOwned(cmd)
		if err != nil || got != want {
			t.Fatalf("settle = %s, %v; want %s", got, err, want)
		}
	}
	messages, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(messages) != 1 || messages[0].To != "lead" || messages[0].Task != "requester-task" {
		t.Fatalf("requester notices = %+v", messages)
	}
}
