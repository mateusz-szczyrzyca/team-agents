package main

// Tests of helper-dump (written first): the worker requests of a task, oldest first, as the harness shows them in
// `agent status` and in the description of a pull request — with their step, their worker and their state.

import "testing"

func TestHelpersOfATaskAreItsOwnRequestsOldestFirst(t *testing.T) {
	st := &State{Helpers: map[string]*Helper{
		"3": {ID: 3, FromTask: "task-a", Step: "1.2", Worker: "glm-1", Status: HelperDone},
		"1": {ID: 1, FromTask: "task-a", Status: HelperQueued},
		"2": {ID: 2, FromTask: "task-b", Step: "2.1", Status: HelperRunning},
	}}

	got := helpersOfTask(st, "task-a")

	if len(got) != 2 || got[0].ID != 1 || got[1].ID != 3 || got[1].Step != "1.2" || got[1].Worker != "glm-1" {
		t.Fatalf("the requests of task-a, oldest first: %+v", got)
	}
	if none := helpersOfTask(st, "task-c"); len(none) != 0 {
		t.Fatalf("a task without requests has none: %+v", none)
	}
}

func TestHelperDumpNeedsATask(t *testing.T) {
	failsWith(t, CmdHelperDumpMissingTask)("", helperDumpTask(" "))
	if err := helperDumpTask("task-a"); err != nil {
		t.Fatalf("a task is enough: %v", err)
	}
}
