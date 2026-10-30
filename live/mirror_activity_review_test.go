package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// replaceMirrorActivity reproduces sync's atomic replacement, rather than an append.
func replaceMirrorActivity(t *testing.T, path, contents string) {
	t.Helper()
	next := path + ".next"
	writeFile(t, next, contents)
	if err := os.Rename(next, path); err != nil {
		t.Fatal(err)
	}
}

// mirrorActivityLine uses distinct timestamps so equal text can still represent distinct events.
func mirrorActivityLine(n int, text string) string {
	return fmt.Sprintf("{\"ts\":\"10:00:%02d\",\"k\":\"text\",\"t\":%q}\n", n, text)
}

// TestMirrorWatcherAtomicTails preserves new events through bounded-tail replacement and partial writes.
func TestMirrorWatcherAtomicTails(t *testing.T) {
	for _, size := range []string{"equal", "larger", "smaller", "no overlap"} {
		t.Run(size, func(t *testing.T) {
			dir := t.TempDir()
			task := &Task{ID: "T@node", Node: "node", Dir: dir}
			path := filepath.Join(dir, "activity.jsonl")
			a, b, c := mirrorActivityLine(1, "past"), mirrorActivityLine(2, "same"), mirrorActivityLine(3, "next")
			if size == "smaller" {
				a = mirrorActivityLine(1, strings.Repeat("old", 50))
			}
			writeFile(t, path, a+b)
			watcher := NewWatcher(dir, filepath.Join(dir, "bus"), 0)
			if got := watcher.newActivity(task, false); len(got) != 0 {
				t.Fatalf("baseline: %+v", got)
			}
			tail := b + c
			if size == "equal" && len(tail) != len(a+b) {
				t.Fatal("equal-size replacement fixture differs in length")
			}
			if size == "larger" {
				tail += mirrorActivityLine(4, "larger tail")
			}
			if size == "no overlap" {
				tail = c
			}
			replaceMirrorActivity(t, path, tail)
			got := watcher.newActivity(task, false)
			want := 1
			if size == "larger" {
				want = 2
			}
			if len(got) != want || got[0].Text != "next" {
				t.Fatalf("replacement: got %+v, want %d new events starting next", got, want)
			}
			if got := watcher.newActivity(task, false); len(got) != 0 {
				t.Fatalf("unchanged tail replayed: %+v", got)
			}
			partial := mirrorActivityLine(5, "completed later")
			replaceMirrorActivity(t, path, tail+strings.TrimSuffix(partial, "\n"))
			if got := watcher.newActivity(task, false); len(got) != 0 {
				t.Fatalf("partial line or prior events replayed: %+v", got)
			}
			replaceMirrorActivity(t, path, tail+partial)
			if got := watcher.newActivity(task, false); len(got) != 1 || got[0].Text != "completed later" {
				t.Fatalf("completed line: %+v", got)
			}
		})
	}
}

// TestMirrorTaskSSEAtomicTail verifies the actual stream uses a replacement-aware cursor.
func TestMirrorTaskSSEAtomicTail(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	path := filepath.Join(dir, "activity.jsonl")
	a, b, c := mirrorActivityLine(1, "past"), mirrorActivityLine(2, "same"), mirrorActivityLine(3, "next")
	writeFile(t, path, a+b)
	_, ts := testServer(t, home)
	br := openStream(t, ts, fmt.Sprintf("/events/task/T1@node-a?offset=%d", len(a+b)))
	replaceMirrorActivity(t, path, b+c)
	got := readEvent(t, br, "act")
	if !strings.Contains(got, "next") || strings.Contains(got, "same") {
		t.Fatalf("replacement event: %s", got)
	}
}

// TestMirrorDashboardConfiguredTimeout checks streamed rows use the same timeout as the initial page.
func TestMirrorDashboardConfiguredTimeout(t *testing.T) {
	home := fixture(t)
	mirrorFixture(t, home)
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), fmt.Sprintf(`{"Version":1,"Nodes":{"node-a":{"LastSeen":%q}}}`, time.Now().Add(-time.Hour).UTC().Format(time.RFC3339Nano)))
	server, ts := testServer(t, home)
	server.cfg.nodeTTL = 48 * time.Hour
	br := openStream(t, ts, "/events/dashboard")
	got := readEvent(t, br, "table")
	if !strings.Contains(got, "worker-1@node-a") || strings.Contains(got, "node offline") {
		t.Fatalf("stream ignored configured timeout: %s", got)
	}
}
