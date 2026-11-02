package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestWatcherPollIgnoresSyntheticInvestigationTeams verifies that Watcher.Poll emits no events
// (new task, status change, or activity) for synthetic investigation team tasks.
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
	for _, ev := range events {
		if strings.Contains(ev.Text, "investigation-team") || strings.Contains(ev.Text, "TTEAM1") {
			t.Errorf("Watcher.Poll unexpectedly emitted event for synthetic team task: %s", ev.Text)
		}
	}
}
