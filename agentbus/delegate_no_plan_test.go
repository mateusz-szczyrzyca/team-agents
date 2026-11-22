package main

// Tests of a run whose role keeps a plan but whose worktree has no plan file (written first): the harness answers
// NO-PLAN, and the request is one of an agent without a plan — the old form is enough, nothing of the four pieces is
// required, no step is recorded and the request waits in the queue as before, also with no worker free.

import (
	"path/filepath"
	"strings"
	"testing"
)

// absentPlanMember is the member m of team T with the members given, whose bus was given a plan file that is not
// there; the harness answers NO-PLAN, as `agent _delegation-check` does for such a file.
func absentPlanMember(t *testing.T, b *Bus, members map[string][3]string) (*Server, *[]string) {
	t.Helper()
	createTeam(t, b, "T", members)
	m := server(b, "m", inTeam("T"))
	m.plan = filepath.Join(t.TempDir(), ".agent", "plan.md")
	m.agentCmd, m.home = "agent", "/home/fake"
	calls := harnessCheck(t, func(string) string { return "NO-PLAN" })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	return m, calls
}

func TestDelegateWithAnAbsentPlanTakesTheOldForm(t *testing.T) {
	b := newBus(t)
	m, calls := absentPlanMember(t, b, map[string][3]string{"m": {"normal", "a", "active"}, "w": {"worker", "z", "sleeping"}})

	chk(t).ok(toolResult(m.delegateFields(fields(t.TempDir()))))

	if len(*calls) != 1 || !strings.Contains((*calls)[0], "_delegation-check --plan "+m.plan) {
		t.Fatalf("the harness should be asked whether the plan is there, got %v", *calls)
	}
	mustWith(t, b, func(st *State) {
		if h := st.Helpers["1"]; h == nil || h.Step != "" {
			t.Fatalf("an old-form request is queued with no step, got %+v", h)
		}
	})
}

func TestDelegateWithAnAbsentPlanRecordsNoStepAndWaitsWithoutAFreeWorker(t *testing.T) {
	b := newBus(t)
	m, _ := absentPlanMember(t, b, map[string][3]string{"m": {"normal", "a", "active"}})

	chk(t).ok(toolResult(m.delegateFields(namedFields(t.TempDir()))))

	mustWith(t, b, func(st *State) {
		if h := st.Helpers["1"]; h == nil || h.Step != "" || h.Status != HelperQueued {
			t.Fatalf("a request the harness did not accept for a step is queued with no step, got %+v", h)
		}
	})
}
