package main

// Tests of a request to a worker of an agent with a plan (written first): such a request names a step of the
// requester's plan (for), its files, its check and what to do; the bus asks the harness's hidden
// `agent _delegation-check`, which reads the plan at the moment of the request and the [plan] check_refuse, and a
// refused request queues nothing and starts no worker. An agent without a plan (no --plan) keeps the old form.

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// harnessCheck is a stand-in for the `agent _delegation-check` of the harness: it answers with the keyword answer,
// empty meaning ACCEPTED and then the step, and it records the arguments it was given.
func harnessCheck(t *testing.T, answer func(step string) string) *[]string {
	t.Helper()
	var calls []string
	old := agentCheckFn
	agentCheckFn = func(agentCmd, home string, args ...string) (AgentCheckVerdict, string, *ProcessError) {
		calls = append(calls, agentCmd+" "+home+" "+strings.Join(args, " "))
		step := ""
		for i, a := range args {
			if a == "--for" && i+1 < len(args) {
				step = args[i+1]
			}
		}
		return AgentCheckYes, answer(step), nil
	}
	t.Cleanup(func() { agentCheckFn = old })
	return &calls
}

// missingPiece returns the check of a refusal with the cause want.
func missingPiece(t *testing.T, want DelegateCause) func(error) {
	return func(e error) { expectToolError(t, e, want) }
}

// toError is an operation's error as a plain error, nil when the operation succeeded.
func toError[C opCause](_ string, err *OpError[C]) error {
	if err == nil {
		return nil
	}
	return err.asError()
}

// asInvestigator makes s an investigator: a stronger model that may also lead, review and delegate.
func asInvestigator(s *Server) { s.role = RoleInvestigator }

// planPath is a plan file of a run that keeps a plan.
func planPath(t *testing.T) string {
	t.Helper()
	plan := filepath.Join(t.TempDir(), "plan.md")
	if err := os.WriteFile(plan, []byte("1. Draft\n[ ] 1.1 Write it\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	return plan
}

// namedFields — a complete request in dir with the four new fields.
func namedFields(dir string) DelegateFields {
	d := fields(dir)
	d.Step, d.Files, d.Check, d.Doing = "1.1", "docs/guide.md", "exists docs/guide.md", "Write the guide."
	return d
}

// acceptedAnswer is what the harness prints for an accepted request. The bus quotes it as it came: the step is a
// block of text, not one line, and the bus does not parse it.
const acceptedAnswer = "ACCEPTED\n" +
	"The step of the requester's plan:\nStep 1.1 Write it\n" +
	"Files: docs/guide.md\nCheck: exists docs/guide.md\n" +
	"Request files: docs/guide.md\nRequest check: exists docs/guide.md\nDo: Write the guide."

func TestDelegateWithAPlanAsksTheHarnessForThePlanAtTheMomentOfTheRequest(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	plan := planPath(t)
	n.plan = plan
	n.agentCmd, n.home = "agent", "/home/fake"
	calls := harnessCheck(t, func(string) string { return acceptedAnswer })
	var spawned []string
	old := agentCmdFn
	agentCmdFn = func(_, _ string, args ...string) { spawned = append(spawned, strings.Join(args, " ")) }
	t.Cleanup(func() { agentCmdFn = old })

	chk(t).ok(toolResult(n.delegateFields(namedFields(t.TempDir()))))

	if len(*calls) != 1 {
		t.Fatalf("the harness should be asked once, got %v", *calls)
	}
	if want := "agent /home/fake _delegation-check --plan " + plan + " --for 1.1 --files docs/guide.md --check exists docs/guide.md --do Write the guide."; (*calls)[0] != want {
		t.Fatalf("harness call = %q, want %q", (*calls)[0], want)
	}
	if len(spawned) != 1 || spawned[0] != "_dispatch-helpers" {
		t.Fatalf("an accepted request dispatches once, got %v", spawned)
	}
	var h *Helper
	mustWith(t, b, func(st *State) { h = st.Helpers["1"] })
	if h == nil || h.Step != "1.1" {
		t.Fatalf("the request should carry its step, got %+v", h)
	}
	for _, want := range []string{strings.TrimPrefix(acceptedAnswer, "ACCEPTED\n")} {
		if !strings.Contains(h.Request, want) {
			t.Fatalf("the worker's request should hold %q:\n%s", want, h.Request)
		}
	}
}

func TestDelegateWithAPlanRefusesEachMissingPieceWithItsOwnCause(t *testing.T) {
	b := newBus(t)
	for _, row := range []struct {
		name  string
		clear func(*DelegateFields)
		check func(error)
	}{
		{"for", func(d *DelegateFields) { d.Step = " " }, missingPiece(t, DelegateMissingFor)},
		{"files", func(d *DelegateFields) { d.Files = "" }, missingPiece(t, DelegateMissingFiles)},
		{"check", func(d *DelegateFields) { d.Check = "" }, missingPiece(t, DelegateMissingCheck)},
		{"do", func(d *DelegateFields) { d.Doing = "\n" }, missingPiece(t, DelegateMissingDo)},
	} {
		t.Run(row.name, func(t *testing.T) {
			n := server(b, "n1")
			n.plan = planPath(t)
			n.agentCmd, n.home = "agent", "/home/fake"
			harnessCheck(t, func(string) string { return acceptedAnswer })
			var spawned []string
			old := agentCmdFn
			agentCmdFn = func(_, _ string, args ...string) { spawned = append(spawned, strings.Join(args, " ")) }
			t.Cleanup(func() { agentCmdFn = old })
			d := namedFields(t.TempDir())
			row.clear(&d)
			row.check(toError(n.delegateFields(d)))
			if len(spawned) != 0 {
				t.Fatalf("a refused request must not dispatch a worker, got %v", spawned)
			}
			mustWith(t, b, func(st *State) {
				if len(st.Helpers) != 0 {
					t.Fatalf("a refused request must not queue anything, got %+v", st.Helpers)
				}
			})
		})
	}
}

func TestDelegateWithAPlanMapsTheHarnessRefusalToItsOwnCause(t *testing.T) {
	b := newBus(t)
	cases := []struct {
		keyword string
		check   func(error)
	}{
		{"MISSING-STEP", func(e error) { expectToolError(t, e, DelegateMissingFor) }},
		{"MISSING-FILES", func(e error) { expectToolError(t, e, DelegateMissingFiles) }},
		{"MISSING-CHECK", func(e error) { expectToolError(t, e, DelegateMissingCheck) }},
		{"MISSING-DO", func(e error) { expectToolError(t, e, DelegateMissingDo) }},
		{"NO-SUCH-STEP", func(e error) { expectToolError(t, e, DelegateNoSuchStep) }},
		{"STEP-CLOSED", func(e error) { expectToolError(t, e, DelegateStepClosed) }},
		{"PATH-OUTSIDE", func(e error) { expectToolError(t, e, DelegatePathOutside) }},
		{"CHECK-REFUSED", func(e error) { expectToolError(t, e, DelegateCheckRefused) }},
		{"PLAN-UNREADABLE", func(e error) { expectToolError(t, e, DelegatePlanUnreadable) }},
	}
	for _, c := range cases {
		t.Run(c.keyword, func(t *testing.T) {
			n := server(b, "n1")
			n.plan = planPath(t)
			n.agentCmd, n.home = "agent", "/home/fake"
			harnessCheck(t, func(string) string { return "REFUSED " + c.keyword + "\nthe request needs something" })
			d := namedFields(t.TempDir())
			if c.keyword == "MISSING-FOR" {
				d.Step = ""
			}
			c.check(toError(n.delegateFields(d)))
		})
	}
}

func TestDelegateRefusalSaysWhatToAdd(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string {
		return "REFUSED NO-SUCH-STEP\nthe request's for 9.9 names no step of your plan: name a step it has"
	})

	_, err := n.delegateFields(namedFields(t.TempDir()))

	if err == nil || !strings.Contains(err.Message, "names no step of your plan") {
		t.Fatalf("the refusal should carry what to add, got %v", err)
	}
}

func TestDelegateWithoutAPlanKeepsTheOldForm(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	calls := harnessCheck(t, func(string) string { return "" })
	var spawned []string
	old := agentCmdFn
	agentCmdFn = func(_, _ string, args ...string) { spawned = append(spawned, strings.Join(args, " ")) }
	t.Cleanup(func() { agentCmdFn = old })

	chk(t).ok(toolResult(n.delegateFields(fields(t.TempDir()))))

	if len(*calls) != 0 {
		t.Fatalf("an agent without a plan asks the harness nothing, got %v", *calls)
	}
	if len(spawned) != 1 || spawned[0] != "_dispatch-helpers" {
		t.Fatalf("the old form dispatches once, got %v", spawned)
	}
	mustWith(t, b, func(st *State) {
		if h := st.Helpers["1"]; h == nil || h.Step != "" {
			t.Fatalf("an old-form request carries no step, got %+v", h)
		}
	})
}

func TestDelegateDefaultHarnessCheckAnswersThePlan(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"

	// The default is the real call of `agent _delegation-check`: `agent` is not there, so the check fails.
	_, err := n.delegateFields(namedFields(t.TempDir()))
	if err == nil || !(errors.Is(err, DelegateCheckFailed.info().class.Sentinel())) {
		t.Fatalf("a harness that cannot answer is a failure of the check, got %v", err)
	}
}

func TestDelegateInvestigatorMayAskAWorkerMayNot(t *testing.T) {
	b := newBus(t)
	var spawned []string
	old := agentCmdFn
	agentCmdFn = func(_, _ string, args ...string) { spawned = append(spawned, strings.Join(args, " ")) }
	t.Cleanup(func() { agentCmdFn = old })
	chk(t).ok(toolResult(server(b, "i1", asInvestigator).delegate("count the files", "src/")))
	if len(spawned) != 1 {
		t.Fatalf("an investigator should be able to delegate, got %v", spawned)
	}
	failsWith(t, DelegateNotNormalTier)(server(b, "w", asWorker).delegate("x", ""))
}

func TestInvestigatorHasTheDelegationTools(t *testing.T) {
	s := server(newBus(t), "i1", asInvestigator)
	names := map[string]bool{}
	for _, tool := range s.tools() {
		if name, ok := tool["name"].(string); ok {
			names[name] = true
		}
	}
	for _, want := range []string{"delegate", "delegation_status", "delegation_feedback"} {
		if !names[want] {
			t.Fatalf("an investigator should have %s, got %v", want, names)
		}
	}
	if names["board_claim"] || names["quiz_start"] {
		t.Fatalf("an investigator should have no board or quiz tools, got %v", names)
	}
}

func TestDelegatePlanFieldsInTheSchema(t *testing.T) {
	var def map[string]any
	for _, t := range collabToolDefs(true, TierNormal) {
		if t["name"] == "delegate" {
			def = t
		}
	}
	schema := def["inputSchema"].(map[string]any)
	props := schema["properties"].(map[string]any)
	for _, want := range []string{"for", "files", "check", "do"} {
		if props[want] == nil {
			t.Fatalf("delegate should declare %s, got %v", want, props)
		}
	}
	if got := strings.Join(schema["required"].([]string), ","); got != "goal,workdir,steps,returns" {
		t.Fatalf("required = %s: the new fields are optional in the schema", got)
	}
}

func TestDelegatePlanArgsThroughTheDispatcher(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })

	chk(t).ok(n.callTool("delegate", map[string]any{
		"goal": "the guide", "workdir": t.TempDir(), "steps": "write", "returns": "the file",
		"for": "1.1", "files": "docs/guide.md", "check": "exists docs/guide.md", "do": "Write the guide.",
	}))
}

func TestHelperStepReadsOlderStates(t *testing.T) {
	b := installState(t, helperBeforeRemote)
	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil || h.Step != "" {
			t.Fatalf("a request recorded before a step existed has none: %+v", h)
		}
	})
}
