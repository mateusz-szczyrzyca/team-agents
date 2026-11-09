package main

// Tests of UNCLEAR through a node (written first): a remote worker's job reports only its state — done, failed or
// stopped — and its result's text. A result that is the worker's questions (STATUS: UNCLEAR or QUESTIONS, or the
// former STATUS: FAILED with Result: UNCLEAR) makes the request unclear, not done or failed, and the requester's
// message says [unclear]; any other result keeps the job's state.

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestRemoteWorkerQuestionsMakeTheRequestUnclear(t *testing.T) {
	for _, c := range []struct {
		name   string
		state  JobState
		result string
		want   HelperStatus
	}{
		{"unclear", JobDone, "STATUS: UNCLEAR\nQuestions:\n- Which directory?", HelperUnclear},
		{"questions", JobDone, "STATUS: QUESTIONS\n- Which directory?", HelperUnclear},
		{"former shape", JobFailed, "STATUS: FAILED\nResult: UNCLEAR: which directory?", HelperUnclear},
		{"a failure", JobFailed, "STATUS: FAILED\nResult: find exited 1", HelperFailed},
		{"a result", JobDone, "STATUS: DONE\nResult: 42 files", HelperDone},
	} {
		t.Run(c.name, func(t *testing.T) {
			b := newBus(t)
			remoteHelperFixture(t, b)
			if err := finishRemoteHelper(b, 1, remoteHelperResult{State: c.state, Node: "node-b", Branch: "agents/r", Result: c.result}); err != nil {
				t.Fatal(err)
			}
			mustWith(t, b, func(st *State) {
				if got := st.Helpers["1"].Status; got != c.want {
					t.Fatalf("H1 = %s, want %s", got, c.want)
				}
			})
			msgs, err := b.messages()
			if err != nil || len(msgs) != 1 {
				t.Fatalf("messages=%+v err=%v", msgs, err)
			}
			var event EventScript // the requester's helper-result notice
			if err := json.Unmarshal(msgs[0].Data, &event); err != nil {
				t.Fatal(err)
			}
			if label := "[" + string(c.want) + "]"; !strings.Contains(msgs[0].Text, label) {
				t.Errorf("the requester's message lacks %s: %q", label, msgs[0].Text)
			}
		})
	}
}
