package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestLogTimestampTiesKeepNewestSourceEntryFirst checks ordering within one timestamp.
func TestLogTimestampTiesKeepNewestSourceEntryFirst(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "logs", "agent.jsonl"), `{"ts":"2026-10-30T08:00:00Z","level":"info","event":"older_tie"}`+"\n"+`{"ts":"2026-10-30T08:00:00Z","level":"info","event":"newer_tie"}`+"\n")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/logs?node=local")
	if !strings.Contains(body, "newer_tie") || strings.Index(body, "older_tie") < strings.Index(body, "newer_tie") {
		t.Fatal("equal timestamp logs lost descending source order")
	}
}

// TestMirrorElapsedUsesCurrentRun checks elapsed time without relying on a local runner process.
func TestMirrorElapsedUsesCurrentRun(t *testing.T) {
	snap := &Snapshot{}
	task := &Task{ID: "T@node-a", Node: "node-a", Status: "running", Created: time.Now().Add(-24 * time.Hour), RunStart: time.Now().Add(-2 * time.Minute)}
	if got := makeRow(snap, task).Since; got != human(time.Since(task.RunStart)) {
		t.Errorf("web elapsed=%q, want current run", got)
	}
	panel := (model{snap: snap}).row(task, 120)
	if !strings.Contains(panel, human(time.Since(task.RunStart))) {
		t.Errorf("panel elapsed uses creation time: %s", panel)
	}
}

// TestTerminalMirrorQuestionsUseAvailableFiles keeps read-only question history visible.
func TestTerminalMirrorQuestionsUseAvailableFiles(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	writeFile(t, filepath.Join(dir, "questions-2.md"), "Earlier mirrored question")
	writeFile(t, filepath.Join(dir, "questions-10.md"), "Latest mirrored question")
	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	task := snap.All["T1@node-a"]
	task.Status = "waiting"
	panel := model{snap: snap, width: 120, height: 40, tab: 2}
	body := panel.detail(task)
	if !strings.Contains(body, "Latest mirrored question") || strings.Index(body, "Earlier mirrored question") < strings.Index(body, "Latest mirrored question") {
		t.Errorf("question files missing or out of numeric order: %s", body)
	}
	if strings.Contains(body, "key a") {
		t.Fatal("read-only mirror questions invite answering on the lead")
	}
}
