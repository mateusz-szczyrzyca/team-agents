package main

import (
	"path/filepath"
	"testing"
)

// TestMirrorWatcherDuplicateRollover preserves new identical events when all old occurrences roll out.
func TestMirrorWatcherDuplicateRollover(t *testing.T) {
	dir := t.TempDir()
	task := &Task{ID: "T@node", Node: "node", Dir: dir}
	path := filepath.Join(dir, "activity.jsonl")
	x := mirrorActivityLine(1, "identical-x")
	y := mirrorActivityLine(2, "new-y")
	writeFile(t, path, x+x)
	watcher := NewWatcher(dir, filepath.Join(dir, "bus"), 0)
	if got := watcher.newActivity(task, false); len(got) != 0 {
		t.Fatalf("baseline emitted events: %+v", got)
	}
	// The producer appended another X then Y to a two-record bounded tail.
	replaceMirrorActivity(t, path, x+y)
	got := watcher.newActivity(task, false)
	if len(got) != 2 || got[0].Text != "identical-x" || got[1].Text != "new-y" {
		t.Fatalf("rollover lost the new identical occurrence: got %+v, want X then Y", got)
	}
	if got := watcher.newActivity(task, false); len(got) != 0 {
		t.Fatalf("unchanged tail replayed events: %+v", got)
	}
}
