package main

import (
	"bufio"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// allEvents returns one value of every event type (used to check kinds and texts).
func allEvents() []busEvent {
	return []busEvent{
		EventChat{Body: "hi"}, EventReserved{Resource: "p"}, EventReleased{Resource: "p"},
		EventAutoReleased{Resource: "p", Holder: "a", Reason: ReleaseAgentGone}, EventForcedRelease{Resource: "p", Holder: "a"},
		EventInvestigationRequested{ID: 1, Status: InvQueued, Question: "q"}, EventInvestigationPromoted{ID: 1, Requester: "a"},
		EventInvestigationStarted{ID: 1, Requester: "a", Question: "q"}, EventInvestigationResumed{ID: 1},
		EventInvestigationWaiting{ID: 1}, EventInvestigationRequeued{ID: 1},
		EventInvestigationFinished{ID: 1, Status: InvDone, Summary: "s"}, EventDecisionRecorded{ID: 1, Decision: "d"},
		EventReviewQueued{ID: 1, Status: InvQueued, PRURL: "u"}, EventTeamCreated{Team: "T", Members: []string{"a"}},
		EventTeamResumed{Team: "T2", Previous: "T", Members: []string{"a"}}, EventTeamFinished{Team: "T"},
		EventCoordinatorClaimed{Coordinator: "a"}, EventCoordinatorTakeover{Coordinator: "a", Previous: "b"},
		EventCoordinatorHandover{From: "a", To: "b"}, EventQuizQuestion{Question: "q", Seconds: 60},
		EventExcludedNotice{Team: "T", Reason: "r"}, EventMemberExcluded{Member: "a", Reason: "r"},
		EventBoardItemAdded{ID: "B1", Level: LevelEasy, Title: "t"}, EventBoardResult{ID: "B1", Title: "t", Result: "r"},
		EventBoardReturned{ID: "B1", Title: "t", Note: "n"}, EventWorkerRequest{ID: 1, Request: "r"},
		EventCheckpoint{Summary: "s"}, EventStopAll{}, EventScript{Notice: NoticeMemberFinished, Body: "b"},
	}
}

func TestEveryEventHasAValidUniqueKindAndAText(t *testing.T) {
	seen := map[MessageKind]bool{}
	for _, ev := range allEvents() {
		k := ev.Kind()
		if !k.Valid() || k == "" {
			t.Errorf("%T: invalid kind %q", ev, k)
		}
		if seen[k] {
			t.Errorf("%T: kind %q used twice", ev, k)
		}
		seen[k] = true
		if strings.TrimSpace(ev.Text()) == "" {
			t.Errorf("%T: empty text", ev)
		}
	}
}

func TestEventTexts(t *testing.T) {
	cases := []struct {
		ev   busEvent
		want string
	}{
		{EventInvestigationFinished{ID: 3, Status: InvDone, Summary: "use DMA"}, "INVESTIGATION #3 finished [done]: use DMA"},
		{EventInvestigationRequeued{ID: 3}, "INVESTIGATION #3 returns to the queue"},
		{EventAutoReleased{Resource: "probe", Holder: "a", Reason: ReleaseAgentFinished}, "AUTO-RELEASE probe (agent a finished)"},
		{EventReserved{Resource: "probe", Note: "flash"}, "RESERVING probe: flash"},
		{EventTeamResumed{Team: "T2", Previous: "T1", Members: []string{"a", "b"}},
			"TEAM T2 RESUMED from T1: a, b — coordinator: to be decided (claim_lead), no quiz, continue from the last checkpoint"},
	}
	for _, c := range cases {
		if got := c.ev.Text(); got != c.want {
			t.Errorf("%T: %q, want %q", c.ev, got, c.want)
		}
	}
}

func TestPostStoresKindDataAndText(t *testing.T) {
	b := newBus(t)
	mustWith(t, b, func(st *State) {
		if _, serr := b.post(st, "agentbus", "all", EventInvestigationFinished{ID: 3, Status: InvFailed, Summary: "timeout"}); serr != nil {
			t.Fatal(serr)
		}
	})
	f, err := os.Open(filepath.Join(b.dir, "messages.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := f.Close(); err != nil {
			t.Error(err)
		}
	}()
	sc := bufio.NewScanner(f)
	if !sc.Scan() {
		t.Fatal("no message written")
	}
	var raw struct {
		Kind string         `json:"kind"`
		Data map[string]any `json:"data"`
		Text string         `json:"text"`
	}
	if err := json.Unmarshal(sc.Bytes(), &raw); err != nil {
		t.Fatal(err)
	}
	if raw.Kind != "investigation_finished" || raw.Data["status"] != "failed" || raw.Data["id"] != float64(3) ||
		raw.Text != "INVESTIGATION #3 finished [failed]: timeout" {
		t.Fatalf("stored message: %+v", raw)
	}
}

func TestMessagesOfOlderVersionsStillRead(t *testing.T) {
	b := newBus(t)
	legacy := `{"id":1,"ts":"2026-10-01 10:00:00","from":"a","to":"all","text":"old style"}` + "\n"
	if err := os.WriteFile(filepath.Join(b.dir, "messages.jsonl"), []byte(legacy), 0o600); err != nil {
		t.Fatal(err)
	}
	msgs, serr := b.messages()
	if serr != nil || len(msgs) != 1 || msgs[0].Kind != "" || msgs[0].Text != "old style" {
		t.Fatalf("got %+v, %v", msgs, serr)
	}
}

func TestInvestigationTransitions(t *testing.T) {
	at := Timestamp{time.Date(2026, 10, 20, 9, 0, 0, 0, time.Local)}
	inv := &Investigation{ID: 5, FromAgent: "a", Question: "why", Status: InvStarting}

	if ev := inv.transition(InvRunning, at); ev.Kind() != KindInvestigationStarted || !inv.HasStarted() {
		t.Fatalf("first running: %T, started=%v", ev, inv.Started)
	}
	if ev := inv.transition(InvWaitingHuman, at); ev.Kind() != KindInvestigationWaiting {
		t.Fatalf("waiting_human: %T", ev)
	}
	if ev := inv.transition(InvRunning, at); ev.Kind() != KindInvestigationResumed {
		t.Fatalf("running again must be a resume: %T", ev)
	}
	if ev := inv.transition(InvQueued, at); ev.Kind() != KindInvestigationRequeued {
		t.Fatalf("queued: %T", ev)
	}
	if ev := inv.transition(InvStarting, at); ev != nil {
		t.Fatalf("starting needs no announcement, got %T", ev)
	}
	inv.Summary = "found it"
	ev := inv.transition(InvDone, at)
	fin, ok := ev.(EventInvestigationFinished)
	if !ok || fin.Status != InvDone || fin.Summary != "found it" || !inv.Finished.Equal(at.Time) {
		t.Fatalf("done: %#v, finished=%v", ev, inv.Finished)
	}
}
