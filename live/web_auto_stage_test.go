package main

import (
	"strings"
	"testing"
)

// What --verify and --autopilot do for a finished coordinator's task (AUTO_STAGE) shows as its row's stage.
func TestRowShowsTheAutomationsStageOfAFinishedTask(t *testing.T) {
	task := &Task{ID: "20261030-100000-lead", Agent: "lead", Status: "done", Dir: t.TempDir(),
		AutoStage: "two reviews: inv-astra (#56), inv-opus (#57)"}
	r := makeRow(&Snapshot{}, task)
	if !strings.Contains(r.Stage, "two reviews: inv-astra (#56), inv-opus (#57)") || !strings.HasPrefix(r.Stage, "⚙") {
		t.Errorf("row stage %q, want the automation's stage after ⚙", r.Stage)
	}
	task.AutoStage = ""
	if r := makeRow(&Snapshot{}, task); strings.HasPrefix(r.Stage, "⚙") {
		t.Errorf("row stage %q without an automation's stage", r.Stage)
	}
}
