package main

import (
	"strings"
	"testing"
	"time"
)

// consultTeam creates a team with the coordinator, two normal members (one asleep), a worker and a paused member.
func consultTeam(t *testing.T, b *Bus) {
	t.Helper()
	createTeam(t, b, "T", map[string][3]string{
		"lead": {"normal", "a", "active"}, "na": {"normal", "b", "sleeping"}, "nb": {"normal", "c", "active"},
		"w": {"worker", "d", "sleeping"}, "p": {"normal", "e", "paused"},
	})
}

func TestConsultCollectsAnswersAndSilenceIsNotAgreement(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	consultTeam(t, b)
	lead, na, nb := server(b, "lead", inTeam("T")), server(b, "na", inTeam("T")), server(b, "nb", inTeam("T"))

	out := chk(t).ok(toolResult(lead.consultStart("1. Which store? a) files b) sqlite — recommend one", 0)))
	if !strings.Contains(out, "C1") || !strings.Contains(out, "na, nb") {
		t.Fatalf("consult_start: %q", out)
	}
	// only the available normal members are asked; the sleeping one is woken by the request
	if wakeCount(woken, "task-na") != 1 || wakeCount(woken, "task-w") != 0 || wakeCount(woken, "task-p") != 0 {
		t.Fatalf("wake-ups: %v", *woken)
	}
	chk(t).ok(toolResult(na.consultReply("C1", "a) — files: simpler, nothing to migrate")))
	res := chk(t).ok(toolResult(lead.consultResults("C1", false)))
	if !strings.Contains(res, "na (") || !strings.Contains(res, "files: simpler") || !strings.Contains(res, "nb: no answer yet") {
		t.Fatalf("results before the limit:\n%s", res)
	}
	// the time limit passes: silence is shown as NO ANSWER, never as agreement
	mustWith(t, b, func(st *State) {
		st.Teams["T"].Consults["C1"].Deadline = Timestamp{time.Now().Add(-time.Minute).Truncate(time.Second)}
	})
	res = chk(t).ok(toolResult(lead.consultResults("C1", true))) // returns at once: the limit passed
	if !strings.Contains(res, "nb: NO ANSWER") || !strings.Contains(res, "not agreement") {
		t.Fatalf("results after the limit:\n%s", res)
	}
	// a late answer is still recorded, marked LATE
	late := chk(t).ok(toolResult(nb.consultReply("C1", "b)")))
	if !strings.Contains(late, "late") {
		t.Fatalf("late reply: %q", late)
	}
	res = chk(t).ok(toolResult(lead.consultResults("C1", false)))
	if !strings.Contains(res, "LATE): b)") {
		t.Fatalf("late answer not marked:\n%s", res)
	}
	// the answers reach the member who asked as messages
	msgs := chk(t).ok(toolResult(lead.read(0)))
	if !strings.Contains(msgs, "CONSULTATION C1 — answer from na") || !strings.Contains(msgs, "answer from nb (LATE)") {
		t.Fatalf("the asker did not get the answers:\n%s", msgs)
	}
}

func TestConsultWaitReturnsWhenEveryoneAnswered(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	consultTeam(t, b)
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.consultStart("q", 30)))
	for _, n := range []string{"na", "nb"} {
		chk(t).ok(toolResult(server(b, n, inTeam("T")).consultReply("C1", "ok from "+n)))
	}
	done := make(chan string, 1)
	go func() {
		out, _ := lead.consultResults("C1", true)
		done <- out
	}()
	select {
	case out := <-done:
		if !strings.Contains(out, "ok from na") || !strings.Contains(out, "ok from nb") {
			t.Fatalf("results:\n%s", out)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("consult_results(wait) did not return although everyone answered")
	}
}

func TestConsultWithNobodyToAsk(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "w": {"worker", "d", "sleeping"}})
	lead := server(b, "lead", inTeam("T"))
	out := chk(t).ok(toolResult(lead.consultStart("q", 0)))
	if !strings.Contains(out, "Nobody to consult") {
		t.Fatalf("consult_start: %q", out)
	}
	failsWith(t, ConsultResultsNoSuchConsult)(lead.consultResults("C1", false))
}

func TestConsultCauses(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	consultTeam(t, b)
	lead, na, w := server(b, "lead", inTeam("T")), server(b, "na", inTeam("T")), server(b, "w", inTeam("T"))
	outsider := server(b, "outsider")

	failsWith(t, ConsultStartMissingText)(lead.consultStart("  ", 0))
	failsWith(t, ConsultStartBadMinutes)(lead.consultStart("q", -1))
	failsWith(t, ConsultStartBadMinutes)(lead.consultStart("q", maxConsultMinutes+1))
	failsWith(t, ConsultStartNotInTeam)(outsider.consultStart("q", 0))
	chk(t).ok(toolResult(lead.consultStart("q", 0)))

	failsWith(t, ConsultReplyMissingArgument)(na.consultReply("C1", ""))
	failsWith(t, ConsultReplyNotInTeam)(outsider.consultReply("C1", "x"))
	failsWith(t, ConsultReplyNoSuchConsult)(na.consultReply("C9", "x"))
	failsWith(t, ConsultReplyNotAsked)(w.consultReply("C1", "x"))
	chk(t).ok(toolResult(na.consultReply("C1", "x")))
	failsWith(t, ConsultReplyAlreadyReplied)(na.consultReply("C1", "y"))

	failsWith(t, ConsultResultsMissingArgument)(lead.consultResults("", false))
	failsWith(t, ConsultResultsNotInTeam)(outsider.consultResults("C1", false))
	failsWith(t, ConsultResultsNoSuchConsult)(lead.consultResults("C9", false))
}

func TestConsultEventsHaveTheirOwnKinds(t *testing.T) {
	seen := map[MessageKind]bool{}
	for _, ev := range allEvents() {
		seen[ev.Kind()] = true
	}
	for _, ev := range []busEvent{EventConsultRequest{ID: "C1", From: "a", Question: "q", Deadline: "12:00"},
		EventConsultReply{ID: "C1", From: "b", Answer: "x"}} {
		if !ev.Kind().Valid() || seen[ev.Kind()] {
			t.Errorf("%T: kind %q invalid or reused", ev, ev.Kind())
		}
		seen[ev.Kind()] = true
		if strings.TrimSpace(ev.Text()) == "" {
			t.Errorf("%T: empty text", ev)
		}
	}
}
