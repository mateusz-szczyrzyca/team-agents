package main

// A helper run (a worker, or an investigator asked by a member) writes to its requester from outside teams: the request
// that started it names the requester's task, and its message is bound to that task's team — also when the member is in
// another open team too, where it must not reach the run of that team.

import (
	"strings"
	"testing"
)

func TestAHelperMessageToItsRequesterIsBoundToTheRequestersTask(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	// the member "codex-1" is in the open teams A and B; the helper serves its request for task-codex-1-b
	twoTeams(t, b)
	mustWith(t, b, func(st *State) {
		st.Agents["codex-1"] = &Presence{Agent: "codex-1", Task: "task-codex-1-b", PID: 4242}
		st.Helpers["1"] = &Helper{ID: 1, From: "codex-1", FromTask: "task-codex-1-b", Repo: "repo",
			Branch: "agents/codex-1", Worktree: "/tmp/wt-codex-1", Request: "list the files", Status: HelperRunning, Worker: "worker-1"}
	})
	helper := server(b, "worker-1")

	chk(t).ok(toolResult(helper.send("codex-1", "the files are a.go and b.go")))

	if got := *woken; len(got) != 1 || got[0] != "task-codex-1-b" {
		t.Fatalf("woken %v, want only the requester's task", got)
	}
	if out := readOutput(t, b, "codex-1", "B"); !strings.Contains(out, "the files are a.go") {
		t.Fatalf("the requester's task did not read the helper's message:\n%s", out)
	}
	if out := readOutput(t, b, "codex-1", "A"); strings.Contains(out, "the files are a.go") {
		t.Fatalf("the member's other team read the helper's message:\n%s", out)
	}
}
