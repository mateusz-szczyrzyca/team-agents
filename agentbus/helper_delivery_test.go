package main

// A helper run (a worker, or an investigator asked by a member) writes to its requester from outside teams: the request
// that started it names the requester's task, and its message is bound to that task's team.

import (
	"strings"
	"testing"
)

func TestAHelperMessageToItsRequesterIsBoundToTheRequestersTask(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	twoTeams(t, b)
	// the request of the member of team B, served by the helper "worker-1"
	mustWith(t, b, func(st *State) {
		st.Agents["lead-b"] = &Presence{Agent: "lead-b", Task: "task-lead-b", PID: 4242}
		st.Helpers["1"] = &Helper{ID: 1, From: "lead-b", FromTask: "task-lead-b", Repo: "repo",
			Branch: "agents/lead-b", Worktree: "/tmp/wt-lead-b", Request: "list the files", Status: HelperRunning, Worker: "worker-1"}
	})
	helper := server(b, "worker-1")

	chk(t).ok(toolResult(helper.send("lead-b", "the files are a.go and b.go")))

	if got := *woken; len(got) != 1 || got[0] != "task-lead-b" {
		t.Fatalf("woken %v, want the requester's task", got)
	}
	if out := readOutput(t, b, "lead-b", "B"); !strings.Contains(out, "the files are a.go") {
		t.Fatalf("the requester did not read the helper's message:\n%s", out)
	}
}
