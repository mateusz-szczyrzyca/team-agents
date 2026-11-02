package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestWatcherPollIgnoresSyntheticInvestigationTeams verifies that Watcher.Poll emits no events
// (new task, status change, or activity) for synthetic investigation team tasks, while real tasks emit normally.
func TestWatcherPollIgnoresSyntheticInvestigationTeams(t *testing.T) {
	home := fixture(t)
	now := time.Now()

	w := NewWatcher(home, filepath.Join(home, "bus"), 0)
	// Initial poll to establish baseline snapshot
	w.Poll()

	// Add synthetic investigation team tasks and a real agent task
	addTask(t, home, "TTEAM1", "AGENT=investigation-team\nKIND=investigation_team\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")
	addTask(t, home, "TREAL", "AGENT=real-agent\nSTATUS=running\nRUN=1\nCREATED="+now.Format(time.RFC3339)+"\n")

	_, events := w.Poll()
	var sawReal bool
	for _, ev := range events {
		if strings.Contains(ev.Text, "real-agent") {
			sawReal = true
		}
		if strings.Contains(ev.Text, "investigation-team") || strings.Contains(ev.Text, "TTEAM1") {
			t.Errorf("Watcher.Poll unexpectedly emitted event for synthetic team task: %s", ev.Text)
		}
	}
	if !sawReal {
		t.Errorf("Watcher.Poll did not emit event for real agent task")
	}

	// Status transition: update synthetic and real tasks
	addTask(t, home, "TTEAM1", "AGENT=investigation-team\nKIND=investigation_team\nSTATUS=done\nRUN=1\n")
	addTask(t, home, "TREAL", "AGENT=real-agent\nSTATUS=done\nRUN=1\n")

	_, events2 := w.Poll()
	sawRealStatus := false
	for _, ev := range events2 {
		if strings.Contains(ev.Text, "real-agent") && strings.Contains(ev.Text, "done") {
			sawRealStatus = true
		}
		if strings.Contains(ev.Text, "investigation-team") || strings.Contains(ev.Text, "TTEAM1") {
			t.Errorf("Watcher.Poll unexpectedly emitted status event for synthetic team task: %s", ev.Text)
		}
	}
	if !sawRealStatus {
		t.Errorf("Watcher.Poll did not emit status transition event for real agent task")
	}
}
