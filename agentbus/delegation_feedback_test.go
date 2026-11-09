package main

// Tests of rating a worker's result (written first): a worker's "done" said nothing of whether its result was right —
// a guessing model looked better than one that asked. The requester rates each done result of its task:
// delegation_feedback(id, verdict, note) — used, corrected (used after fixing it) or discarded, a note required for
// the last two. Its next request waits until the done results of its current task are rated; results of its earlier
// tasks (made before ratings existed) do not hold it.

import "testing"

// doneResult — H<id>, a done result of task's request by from
func doneResult(t *testing.T, b *Bus, id int64, from, task string) {
	t.Helper()
	mustWith(t, b, func(st *State) {
		if st.NextHelperID < id {
			st.NextHelperID = id
		}
		st.Helpers[helperKey(id)] = &Helper{ID: id, From: from, FromTask: task, Worker: "w", Request: "Goal: count",
			Status: HelperDone, Summary: "STATUS: DONE\nResult: 42"}
	})
}

func TestDelegationFeedbackRecordsAVerdict(t *testing.T) {
	b := newBus(t)
	doneResult(t, b, 1, "n1", "task-n1")
	doneResult(t, b, 2, "n1", "task-n1")
	n := server(b, "n1")
	chk(t).ok(toolResult(n.delegationFeedback("H1", "used", "")))
	chk(t).ok(toolResult(n.delegationFeedback("H2", "corrected", "the count left out hidden files")))
	mustWith(t, b, func(st *State) {
		h1, h2 := st.Helpers[helperKey(1)], st.Helpers[helperKey(2)]
		if h1.Verdict != VerdictUsed || h2.Verdict != VerdictCorrected || h2.VerdictNote != "the count left out hidden files" {
			t.Fatalf("H1 = %s, H2 = %s %q", h1.Verdict, h2.Verdict, h2.VerdictNote)
		}
	})
}

func TestDelegationFeedbackRefusals(t *testing.T) {
	b := newBus(t)
	doneResult(t, b, 1, "n1", "task-n1")
	mustWith(t, b, func(st *State) {
		st.Helpers[helperKey(2)] = &Helper{ID: 2, From: "n1", FromTask: "task-n1", Status: HelperUnclear}
	})
	n := server(b, "n1")
	failsWith(t, DelegationFeedbackInvalidVerdict)(n.delegationFeedback("H1", "great", ""))
	failsWith(t, DelegationFeedbackMissingNote)(n.delegationFeedback("H1", "discarded", " "))
	failsWith(t, DelegationFeedbackNoSuchRequest)(n.delegationFeedback("H9", "used", ""))
	failsWith(t, DelegationFeedbackNotYours)(server(b, "n2").delegationFeedback("H1", "used", ""))
	failsWith(t, DelegationFeedbackNotDone)(n.delegationFeedback("H2", "used", "")) // unclear: answered, not rated
	chk(t).ok(toolResult(n.delegationFeedback("H1", "used", "")))
	failsWith(t, DelegationFeedbackAlreadyRated)(n.delegationFeedback("H1", "discarded", "changed my mind"))
}

func TestDelegateWaitsForTheRatingsOfItsTasksResults(t *testing.T) {
	b := newBus(t)
	doneResult(t, b, 1, "n1", "task-n1")
	doneResult(t, b, 2, "n1", "an-earlier-task") // made before ratings existed, or in another task: no hold
	n := server(b, "n1")
	failsWith(t, DelegateUnratedResult)(n.delegateFields(fields(t.TempDir())))
	chk(t).ok(toolResult(n.delegationFeedback("H1", "discarded", "the wrong directory was counted")))
	chk(t).ok(toolResult(n.delegateFields(fields(t.TempDir()))))
}

func TestDelegationFeedbackToolForNormalModels(t *testing.T) {
	found := map[Tier]bool{}
	for _, tier := range []Tier{TierNormal, TierWorker} {
		for _, tl := range collabToolDefs(true, tier) {
			if tl["name"] == "delegation_feedback" {
				found[tier] = true
			}
		}
	}
	if !found[TierNormal] || found[TierWorker] {
		t.Fatalf("delegation_feedback offered: %v (want normal models only, as delegate)", found)
	}
}
