package main

// Tests of the structured worker request (written first): requests H1091 and H1093 came back UNCLEAR — they had
// paths and commands, but rested on the coordinator's own words ("stale URLs", "cap", "relevant R12 sections"). The
// delegate tool takes fields the bus checks: goal, workdir (absolute; existing, for a local agent), steps and
// returns; a harness identifier (a round R12, a board item B80, a request H1091) used in them needs its definition in
// terms. The bus composes the worker's request from the fields.

import (
	"strings"
	"testing"
)

// fields — a complete request in dir
func fields(dir string) DelegateFields {
	return DelegateFields{
		Goal:    "Count the Go files of the agent module",
		Workdir: dir,
		Steps:   "find agent -name '*.go' | wc -l",
		Returns: "the number, and the command's output",
	}
}

func TestDelegateFieldsRequired(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	dir := t.TempDir()
	for name, f := range map[string]func(*DelegateFields){
		"goal":    func(d *DelegateFields) { d.Goal = " " },
		"workdir": func(d *DelegateFields) { d.Workdir = "" },
		"steps":   func(d *DelegateFields) { d.Steps = "" },
		"returns": func(d *DelegateFields) { d.Returns = "\n" },
	} {
		d := fields(dir)
		f(&d)
		t.Run(name, func(t *testing.T) { failsWith(t, DelegateMissingField)(n.delegateFields(d)) })
	}
}

func TestDelegateWorkdirMustBeAnAbsoluteExistingDirectory(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	failsWith(t, DelegateWorkdirInvalid)(n.delegateFields(fields("agent")))
	failsWith(t, DelegateWorkdirInvalid)(n.delegateFields(fields("/no/such/directory")))
	chk(t).ok(toolResult(n.delegateFields(fields(t.TempDir()))))
}

func TestDelegateWorkdirOfARemoteMemberIsOnItsNode(t *testing.T) {
	b := newBus(t)
	r := server(b, "r1", asRemote("node2"))
	chk(t).ok(toolResult(r.delegateFields(fields("/home/node2/worktrees/x"))))   // not on this machine: not looked for
	failsWith(t, DelegateWorkdirInvalid)(r.delegateFields(fields("worktrees/x"))) // still absolute
}

func TestDelegateHarnessIdentifiersNeedDefinitions(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	d := fields(t.TempDir())
	d.Goal = "Inventory what R11 merged and the scope of R12"
	d.Returns = "the totals for H1091"
	failsWith(t, DelegateUndefinedTerm)(n.delegateFields(d))
	d.Terms = "R11: the previous round, merged as PR #71\nR12: the next round"
	failsWith(t, DelegateUndefinedTerm)(n.delegateFields(d)) // H1091 still undefined
	d.Terms += "\nH1091: an earlier request whose totals were questioned"
	chk(t).ok(toolResult(n.delegateFields(d)))
	d2 := fields(t.TempDir())
	d2.Steps = "gh pr view 71 --json files; read PR71's body" // PR71 is no harness identifier
	chk(t).ok(toolResult(n.delegateFields(d2)))
}

func TestDelegateFieldsComposeTheWorkersRequest(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	dir := t.TempDir()
	d := fields(dir)
	d.Terms = "module: a directory with a go.mod"
	chk(t).ok(toolResult(n.delegateFields(d)))
	var h *Helper
	mustWith(t, b, func(st *State) { h = st.Helpers[helperKey(1)] })
	text := h.Request + "\n" + h.Context
	for _, want := range []string{d.Goal, dir, d.Steps, d.Returns, d.Terms} {
		if !strings.Contains(text, want) {
			t.Errorf("the worker's request lacks %q:\n%s", want, text)
		}
	}
}

func TestDelegateToolTakesTheFields(t *testing.T) {
	var def map[string]any
	for _, tl := range collabToolDefs(true, TierNormal) {
		if tl["name"] == "delegate" {
			def = tl
		}
	}
	if def == nil {
		t.Fatal("no delegate tool")
	}
	schema, ok := def["inputSchema"].(map[string]any)
	if !ok {
		t.Fatalf("delegate has no input schema: %v", def)
	}
	req, ok := schema["required"].([]string)
	if !ok {
		t.Fatalf("delegate's schema has no required list: %v", schema)
	}
	if got := strings.Join(req, ","); got != "goal,workdir,steps,returns" {
		t.Errorf("required = %s, want goal,workdir,steps,returns", got)
	}
}
