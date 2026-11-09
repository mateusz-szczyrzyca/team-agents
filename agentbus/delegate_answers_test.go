package main

// Tests of answering an unclear request (written first): a worker that could not do H1 without answers ended it
// unclear, its questions its result. The requester answers them with a new request — answers_to "H1" and answers —
// which the bus accepts only for its own unclear request, not answered before; the worker of the new request gets the
// first request, the questions and the answers. A harness identifier in the answers needs its definition too.

import (
	"strings"
	"testing"
)

// unclearRequest — H1, n1's request that its worker ended unclear with questions
func unclearRequest(t *testing.T, b *Bus) {
	t.Helper()
	mustWith(t, b, func(st *State) {
		st.NextHelperID = 1
		st.Helpers[helperKey(1)] = &Helper{ID: 1, From: "n1", Request: "Goal: count the files", Status: HelperUnclear,
			Summary: "STATUS: UNCLEAR\nQuestions:\n- Which directory holds the files?"}
	})
}

// answering — a complete request answering H1
func answering(t *testing.T) DelegateFields {
	t.Helper()
	d := fields(t.TempDir())
	d.AnswersTo, d.Answers = "H1", "The files are under agent/; hidden files do not count."
	return d
}

func TestDelegateAnswersAnUnclearRequest(t *testing.T) {
	b := newBus(t)
	unclearRequest(t, b)
	n := server(b, "n1")
	chk(t).ok(toolResult(n.delegateFields(answering(t))))
	mustWith(t, b, func(st *State) {
		first, next := st.Helpers[helperKey(1)], st.Helpers[helperKey(2)]
		if next == nil || first.AnsweredBy != 2 || next.FollowsUp != 1 {
			t.Fatalf("H1 answered by %d, H2 = %+v", first.AnsweredBy, next)
		}
		text := next.Request + "\n" + next.Context
		for _, want := range []string{"Goal: count the files", "Which directory holds the files?", "under agent/"} {
			if !strings.Contains(text, want) {
				t.Errorf("the worker's request lacks %q:\n%s", want, text)
			}
		}
	})
}

func TestDelegateAnswersNeedsAnswers(t *testing.T) {
	b := newBus(t)
	unclearRequest(t, b)
	d := answering(t)
	d.Answers = " "
	failsWith(t, DelegateMissingField)(server(b, "n1").delegateFields(d))
}

func TestDelegateAnswersOnlyAnUnclearRequestOfOneselfNotAnsweredYet(t *testing.T) {
	b := newBus(t)
	unclearRequest(t, b)
	failsWith(t, DelegateAnswersToInvalid)(server(b, "n2").delegateFields(answering(t))) // another requester's
	n := server(b, "n1")
	d := answering(t)
	d.AnswersTo = "H9"
	failsWith(t, DelegateAnswersToInvalid)(n.delegateFields(d)) // none such
	chk(t).ok(toolResult(n.delegateFields(answering(t))))
	failsWith(t, DelegateAnswersToInvalid)(n.delegateFields(answering(t))) // answered already: by H2
	mustWith(t, b, func(st *State) { st.Helpers[helperKey(1)].Status, st.Helpers[helperKey(1)].AnsweredBy = HelperDone, 0 })
	failsWith(t, DelegateAnswersToInvalid)(n.delegateFields(answering(t))) // not unclear
}

func TestDelegateAnswersHarnessIdentifiersNeedDefinitions(t *testing.T) {
	b := newBus(t)
	unclearRequest(t, b)
	d := answering(t)
	d.Answers = "Count what R11 merged."
	failsWith(t, DelegateUndefinedTerm)(server(b, "n1").delegateFields(d))
	d.Terms = "R11: the previous round, merged as PR #71"
	chk(t).ok(toolResult(server(b, "n1").delegateFields(d)))
}
