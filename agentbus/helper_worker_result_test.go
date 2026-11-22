package main

// Tests of the request a worker serves: the bus keeps the paths the worker may write, the check of the result and what
// to do next to the step, so that the harness can check the result before anything of it reaches the requester; a
// state written before those fields existed still loads.

import (
	_ "embed"
	"encoding/json"
	"strings"
	"testing"
)

// helperBeforeWorkerResult is a helper record as the bus wrote it while a request carried its step and nothing of its
// files, check or to do.
//
//go:embed testdata/state-before-worker-result.json
var helperBeforeWorkerResult []byte

func TestHelperStateBeforeWorkerResultLoads(t *testing.T) {
	b := installState(t, helperBeforeWorkerResult)
	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil {
			t.Fatal("the request should load")
		}
		if len(h.Files) != 0 || h.Check != "" || h.Doing != "" {
			t.Fatalf("a request written before these fields existed should load without them: %+v", h)
		}
		if h.Step != "1.1" {
			t.Fatalf("the step it had should keep: %+v", h)
		}
	})
}

func TestHelperNextCarriesTheFilesTheCheckAndTheToDoOfARequest(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	chk(t).ok(toolResult(n.delegateFields(namedFields(t.TempDir()))))

	var printed []byte
	mustWith(t, b, func(st *State) {
		if takeLocalHelper(st, "w1", nil) == nil {
			t.Fatal("helper-next should take the queued request")
		}
		h := st.Helpers["1"]
		if h == nil {
			t.Fatal("the request should be there")
		}
		var err error
		printed, err = json.Marshal(h)
		if err != nil {
			t.Fatal(err)
		}
	})
	for _, want := range []string{`"files":["docs/guide.md"]`, `"check":"exists docs/guide.md"`, `"doing":"Write the guide."`} {
		if !strings.Contains(string(printed), want) {
			t.Fatalf("the request the worker takes should carry %s: %s", want, printed)
		}
	}
}

func TestHelperDumpCarriesTheFilesTheCheckAndTheToDoOfARequest(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.plan = planPath(t)
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	chk(t).ok(toolResult(n.delegateFields(namedFields(t.TempDir()))))

	var listed []*Helper
	mustWith(t, b, func(st *State) {
		for _, h := range sortedHelpers(st) {
			listed = append(listed, h)
		}
	})
	if len(listed) != 1 {
		t.Fatalf("one request expected, got %d", len(listed))
	}
	h := listed[0]
	if len(h.Files) != 1 || h.Files[0] != "docs/guide.md" || h.Check != "exists docs/guide.md" || h.Doing != "Write the guide." {
		t.Fatalf("the dump should carry the request's files, check and to do: %+v", h)
	}
}

func TestAnOldFormRequestCarriesNoFilesCheckOrToDo(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	n.agentCmd, n.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	// No plan file: the run sends the old form, which is a look-up.
	chk(t).ok(toolResult(n.delegateFields(fields(t.TempDir()))))

	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil {
			t.Fatal("the request should be queued")
		}
		if len(h.Files) != 0 || h.Check != "" || h.Doing != "" {
			t.Fatalf("the old form should carry none of them: %+v", h)
		}
	})
}
