package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestMirrorWatcherTerminalActivityDrain requires activity synced after the
// terminal status transition to remain visible to dashboard/terminal watchers.
func TestMirrorWatcherTerminalActivityDrain(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	w := NewWatcher(home, filepath.Join(home, "bus"), 0)
	w.Poll()
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"done","RUN":"3","CREATED":"2026-10-23T08:00:00Z","RUN_START":"2026-10-23T09:00:00Z","JOB_ID":"remote-1"}`)
	w.Poll()
	initial := "{\"ts\":\"10:00:07\",\"k\":\"cmd\",\"t\":\"mirrored activity\"}\n"
	final := "{\"ts\":\"10:00:08\",\"k\":\"text\",\"t\":\"final mirrored event\"}\n"
	if err := os.WriteFile(filepath.Join(dir, "activity.jsonl"), []byte(initial+final), 0o600); err != nil {
		t.Fatal(err)
	}
	_, events := w.Poll()
	for _, event := range events {
		if strings.Contains(event.Text, "final mirrored event") {
			return
		}
	}
	t.Fatalf("late terminal activity was not drained: %+v", events)
}

// TestMirrorActivityByteCapDiagnosticKey verifies normal truncation does not retain fragment keys permanently.
func TestMirrorActivityByteCapDiagnosticKey(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "activity.jsonl")
	cursor := &mirrorActivityCursor{}
	tail := ""
	for i := 0; i < 500; i++ {
		tail += mirrorActivityLine(i, strings.Repeat(fmt.Sprintf("%06d", i), 95))
	}
	tail = tail[len(tail)-262144:]
	writeFile(t, path, tail)
	cursor.read(path)
	for i := 500; i < 600; i++ {
		tail += mirrorActivityLine(i, strings.Repeat(fmt.Sprintf("%06d", i), 95))
		if len(tail) > 262144 {
			tail = tail[len(tail)-262144:]
		}
		writeFile(t, path, tail)
		cursor.read(path)
	}
	entries := 0
	warnedOnce.Range(func(key, _ any) bool {
		if s, ok := key.(string); ok && strings.HasPrefix(s, "activity:"+path+":") {
			entries++
		}
		return true
	})
	if entries > 1 {
		t.Errorf("normal truncation diagnostics grow without bound: %d retained keys", entries)
	}
}

// TestMirrorWaitingGuidance checks that local-only answer instructions
// are not emitted for a display-only mirrored task.
func TestMirrorWaitingGuidance(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	w := NewWatcher(home, filepath.Join(home, "bus"), 0)
	w.Poll()
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"waiting","JOB_ID":"remote-1"}`)
	_, events := w.Poll()
	for _, event := range events {
		if strings.Contains(event.Text, "agent answer") || strings.Contains(event.Text, "key a") {
			t.Errorf("mirror event gives unusable local answer instructions: %q", event.Text)
		}
	}
}
