package main

import (
	"path/filepath"
	"strings"
	"testing"
)

// A sleeping member of an open team shows what it waits for: the board items others hold (who, what, their latest
// progress) and those nobody has taken yet; finished items and its own are not among them.
func TestASleepingMemberShowsWhatItWaitsFor(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "tasks", "W1", "meta.env"),
		"ID=W1\nTYPE=codex\nREPO_NAME=repo\nBRANCH=agents/codex-1/x\nCREATED=2026-10-30T16:45:43+01:00\nAGENT=codex-1\nSTATUS=sleeping\nRUN=8\nTEAM=W-team\n")
	writeFile(t, filepath.Join(home, "bus", "state.json"), `{"teams": {"W-team": {"id": "W-team", "lead": "codex-1", "done": false,
	  "members": {"codex-1": {"state": "sleeping", "tier": "normal", "task": "W1"}},
	  "board": {
	    "B36": {"id": "B36", "title": "Native ARM full gate", "status": "claimed", "assignee": "gemini-2@rpi5-x", "progress": "lint-shell running"},
	    "B37": {"id": "B37", "title": "Lint fix", "status": "todo", "for": "gemini-1"},
	    "B5": {"id": "B5", "title": "Old item", "status": "done", "assignee": "gemini-1"},
	    "B40": {"id": "B40", "title": "Its own item", "status": "claimed", "assignee": "codex-1"}}}}}`)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	i := strings.Index(body, "waits for:")
	if i < 0 {
		t.Fatalf("no \"waits for:\" on the dashboard")
	}
	waits := body[i:]
	if j := strings.Index(waits, "<"); j >= 0 {
		waits = waits[:j]
	}
	for _, want := range []string{"B36 gemini-2@rpi5-x: Native ARM full gate (lint-shell running)", "B37 not taken yet (for gemini-1): Lint fix"} {
		if !strings.Contains(waits, want) {
			t.Errorf("%q lacks %q", waits, want)
		}
	}
	for _, no := range []string{"Old item", "Its own item"} {
		if strings.Contains(waits, no) {
			t.Errorf("%q names %q", waits, no)
		}
	}
}
