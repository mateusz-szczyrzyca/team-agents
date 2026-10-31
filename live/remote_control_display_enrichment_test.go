package main

import (
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
)

// TestTerminalCountsAnswerableMirror includes remote questions when the panel enables controls.
func TestTerminalCountsAnswerableMirror(t *testing.T) {
	home, _ := remoteControlServer(t)
	m := newModel(config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", remoteControls: true})
	found := false // the waiting list must include the answerable mirrored task
	for _, task := range m.snap.Waiting() {
		if task.ID == "remote@node" {
			found = true
		}
	}
	if !found {
		t.Fatalf("remote waiting task omitted: CanControl=%v Round=%d", m.snap.All["remote@node"].CanControl, m.snap.All["remote@node"].Round)
	}
}

// TestProxyQuestionsTabReadsMirror shows the real questions under the new answer link.
func TestProxyQuestionsTabReadsMirror(t *testing.T) {
	home, ts := remoteControlServer(t)
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"), "ID=T_PROXY\nAGENT=worker@node\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")
	code, body := get(t, ts, "/task/T_PROXY?tab=questions")
	if code != http.StatusOK || !strings.Contains(body, "Proceed?") {
		t.Fatalf("proxy questions body omits mirrored question: status=%d", code)
	}
}

// TestTerminalRefreshPreservesRawBaseline enriches every refresh without creating false status transitions.
func TestTerminalRefreshPreservesRawBaseline(t *testing.T) {
	home, _ := remoteControlServer(t)
	dir := filepath.Join(home, "bus", "remote", "node", "remote")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker","STATUS":"running","RUN":"1","JOB_ID":"j-remote"}`)
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), `{"Version":1,"Jobs":[{"Request":{"ID":"j-remote"},"Node":"node","TaskID":"remote","State":"waiting"}]}`)
	m := newModel(config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", remoteControls: true})
	for round := 3; round <= 4; round++ {
		writeFile(t, filepath.Join(dir, fmt.Sprintf("questions-%d.md", round)), "## 1. Proceed?\n**Verdict:**\n")
		next, _ := m.Update(tickMsg{})
		m = next.(model)
		view := m.snap.All["remote@node"]
		raw := m.w.prev.All["remote@node"]
		if !view.CanControl || view.Round != round || view.Status != "waiting" {
			t.Fatalf("refresh omitted controls: %+v", view)
		}
		if raw == view || raw.CanControl || raw.Round != 0 || raw.Status != "running" {
			t.Fatal("display enrichment changed the watcher baseline")
		}
		for _, event := range m.events {
			if event.Kind == "status" && strings.Contains(event.Text, "remote") {
				t.Fatalf("display-only enrichment emitted a task transition: %+v", event)
			}
		}
	}
}
