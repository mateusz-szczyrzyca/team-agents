package main

// Tests of the gate at board_done (written first): an agent's "gate PASS" could not be checked. A local member whose
// item's work changed code submits its result only when the harness finds a recorded gate pass of its HEAD
// (agent _board_done_check: exit 3 is "no"); a gate_exception lets the result through marked not verified; a harness
// that cannot answer lets it through, logged; a remote member, or an agent without a task directory, is not checked.

import (
	"strings"
	"testing"
)

// fakeAgentCheck replaces the harness's check for a test: it answers verdict (with out) and records the arguments of
// every call.
func fakeAgentCheck(t *testing.T, verdict AgentCheckVerdict, out string) *[][]string {
	t.Helper()
	calls := &[][]string{}
	orig := agentCheckFn
	agentCheckFn = func(_, _ string, args ...string) (AgentCheckVerdict, string, *ProcessError) {
		*calls = append(*calls, args)
		if verdict == AgentCheckUnknown {
			return verdict, out, processErr("agent", nil, "the fake harness cannot answer")
		}
		return verdict, out, nil
	}
	t.Cleanup(func() { agentCheckFn = orig })
	return calls
}

// withTask makes a test server a local agent of the task dir, with an agent command: one the gate check applies to.
func withTask(dir string) func(*Server) {
	return func(s *Server) { s.taskDir, s.agentCmd = dir, "agent" }
}

// asRemote makes a test server a remote member, working on node n.
func asRemote(n string) func(*Server) { return func(s *Server) { s.remote = n } }

// claimedItem — a team whose member m claimed B1; m built with opts.
func claimedItem(t *testing.T, opts ...func(*Server)) (*Bus, *Server) {
	t.Helper()
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead := server(b, "lead", inTeam("T"))
	m := server(b, "m", append([]func(*Server){inTeam("T")}, opts...)...)
	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.boardAdd("parser", "write the parser", "normal")))
	chk(t).ok(toolResult(m.boardClaim("B1")))
	return b, m
}

// itemOf — B1 as the bus holds it.
func itemOf(t *testing.T, b *Bus) BoardItem {
	t.Helper()
	var it BoardItem
	mustWith(t, b, func(st *State) { it = *st.Teams["T"].Board["B1"] })
	return it
}

func TestBoardDoneRefusedWithoutAGatePassOfHEAD(t *testing.T) {
	calls := fakeAgentCheck(t, AgentCheckNo, "This item's work changes code: run agent gate")
	b, m := claimedItem(t, withTask("/home/x/agents/tasks/T1"))
	failsWith(t, BoardDoneGateNotPassed)(m.boardDoneWith("B1", "done", ""))
	if len(*calls) != 1 || strings.Join((*calls)[0], " ") != "_board_done_check T1" {
		t.Fatalf("harness calls = %v, want one: _board_done_check T1", *calls)
	}
	if it := itemOf(t, b); it.Status == BoardDone {
		t.Fatalf("B1 is done although its gate did not pass")
	}
}

func TestBoardDoneWithAGatePassOfHEAD(t *testing.T) {
	fakeAgentCheck(t, AgentCheckYes, "PASS at abc1234 on a clean tree")
	b, m := claimedItem(t, withTask("/home/x/agents/tasks/T1"))
	chk(t).ok(toolResult(m.boardDoneWith("B1", "parser done", "")))
	if it := itemOf(t, b); it.Status != BoardDone || it.Result != "parser done" {
		t.Fatalf("B1 = %s %q, want done with its result", it.Status, it.Result)
	}
}

func TestBoardDoneWhenTheHarnessCannotAnswer(t *testing.T) {
	fakeAgentCheck(t, AgentCheckUnknown, "")
	b, m := claimedItem(t, withTask("/home/x/agents/tasks/T1"))
	chk(t).ok(toolResult(m.boardDoneWith("B1", "parser done", "")))
	if it := itemOf(t, b); it.Status != BoardDone {
		t.Fatalf("B1 is not done: a harness that cannot answer must not hold the team")
	}
}

func TestBoardDoneGateExceptionMarksTheResultNotVerified(t *testing.T) {
	calls := fakeAgentCheck(t, AgentCheckNo, "no gate pass")
	b, m := claimedItem(t, withTask("/home/x/agents/tasks/T1"))
	chk(t).ok(toolResult(m.boardDoneWith("B1", "parser done", "TestFlaky fails on main too")))
	if len(*calls) != 0 {
		t.Fatalf("the harness was asked although the result came with an exception: %v", *calls)
	}
	it := itemOf(t, b)
	if it.Status != BoardDone || !strings.HasPrefix(it.Result, "⚠ gate not verified — TestFlaky fails on main too") ||
		!strings.Contains(it.Result, "parser done") {
		t.Fatalf("B1 = %s %q, want done, marked not verified, its result kept", it.Status, it.Result)
	}
}

func TestBoardDoneNotCheckedForARemoteMemberOrWithoutATask(t *testing.T) {
	calls := fakeAgentCheck(t, AgentCheckNo, "no gate pass")
	_, remote := claimedItem(t, withTask("/home/x/agents/tasks/T1"), asRemote("node2"))
	chk(t).ok(toolResult(remote.boardDoneWith("B1", "done", "")))
	_, taskless := claimedItem(t)
	chk(t).ok(toolResult(taskless.boardDoneWith("B1", "done", "")))
	if len(*calls) != 0 {
		t.Fatalf("the harness was asked for a remote member or an agent without a task: %v", *calls)
	}
}

func TestAgentCheckVerdictsFromExitStatus(t *testing.T) {
	for _, c := range []struct {
		script string
		want   AgentCheckVerdict
	}{
		{"echo PASS", AgentCheckYes},
		{"echo 'run agent gate'; exit 3", AgentCheckNo},
		{"echo 'unbound variable'; exit 1", AgentCheckUnknown}, // a failing harness is not a refusal
	} {
		got, out, _ := runAgentCheck("/bin/sh", t.TempDir(), "-c", c.script)
		if got != c.want {
			t.Errorf("%q: verdict %v (output %q), want %v", c.script, got, out, c.want)
		}
	}
}
