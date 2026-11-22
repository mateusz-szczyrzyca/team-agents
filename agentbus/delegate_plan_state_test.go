package main

// Tests of the plan a run keeps: the bus's subcommands `helper-next` and `helper-get` carry the request's step, and a
// bus state written before the step field existed still loads with no step.

import (
	_ "embed"
	"encoding/json"
	"strings"
	"testing"
)

// helperBeforeStep is an actual helper record as the bus wrote it before a request carried a step.
//
//go:embed testdata/state-before-plan-step.json
var helperBeforeStep []byte

func TestHelperStateBeforePlanStepLoads(t *testing.T) {
	b := installState(t, helperBeforeStep)
	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil || h.Step != "" || h.Request != "Count the Go files of the agent module" {
			t.Fatalf("the request written before a step existed should load unchanged: %+v", h)
		}
	})
}

func TestHelperNextCarriesTheStepOfARequest(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	chk(t).ok(toolResult(n.delegateFields(namedFields(t.TempDir()))))

	var got map[string]any
	mustWith(t, b, func(st *State) {
		h := takeLocalHelper(st, "w1", nil)
		if h == nil {
			t.Fatal("helper-next should take the queued request")
		}
		got = map[string]any{"step": h.Step, "request": h.Request}
	})
	if got["step"] != "1.1" {
		t.Fatalf("helper-next should carry the step, got %v", got)
	}
	request, _ := got["request"].(string)
	if !strings.HasPrefix(request, "The step of the requester's plan:") {
		t.Fatalf("the worker's request should start with the step, got %q", request)
	}
}

func TestHelperGetCarriesTheStepOfARequest(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	chk(t).ok(toolResult(n.delegateFields(namedFields(t.TempDir()))))

	// helper-get prints the request it finds; the record it prints carries the step.
	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil || h.Step != "1.1" {
			t.Fatalf("helper-get's request should carry the step, got %+v", h)
		}
		printed, err := json.Marshal(h)
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(string(printed), `"step":"1.1"`) {
			t.Fatalf("the printed request should carry the step: %s", printed)
		}
	})
}
