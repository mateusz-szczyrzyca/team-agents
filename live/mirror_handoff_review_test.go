package main

import (
	"html"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// TestMirrorPageStreamHandoff preserves events replaced after HTML rendering but before SSE initialization.
func TestMirrorPageStreamHandoff(t *testing.T) {
	for _, mode := range []string{"equal", "larger", "smaller", "no overlap", "partial", "empty"} {
		t.Run(mode, func(t *testing.T) {
			home := fixture(t)
			dir := mirrorFixture(t, home)
			path := filepath.Join(dir, "activity.jsonl")
			a, b, c := mirrorActivityLine(1, "past"), mirrorActivityLine(2, "same"), mirrorActivityLine(3, "next")
			if mode == "smaller" {
				a = mirrorActivityLine(1, strings.Repeat("past", 100))
			}
			initial := a + b
			if mode == "partial" {
				initial += strings.TrimSuffix(c, "\n")
			}
			if mode == "empty" {
				initial = ""
			}
			writeFile(t, path, initial)
			_, ts := testServer(t, home)
			_, body := get(t, ts, "/task/T1@node-a")
			match := regexp.MustCompile(`sse-connect="([^"]+)"`).FindStringSubmatch(body)
			if len(match) != 2 {
				t.Fatal("activity page has no stream URL")
			}
			tail := b + c
			if mode == "larger" {
				tail += mirrorActivityLine(4, "extra")
			}
			if mode == "no overlap" || mode == "empty" {
				tail = c
			}
			replaceMirrorActivity(t, path, tail)
			br := openStream(t, ts, html.UnescapeString(match[1]))
			// A later event makes a broken handoff fail an assertion instead of waiting for a timeout.
			replaceMirrorActivity(t, path, tail+mirrorActivityLine(5, "sentinel"))
			if got := readEvent(t, br, "act"); !strings.Contains(got, "next") || strings.Contains(got, "same") {
				t.Fatalf("handoff lost the first unseen event: %s", got)
			}
		})
	}
}

// TestMirrorWatcherPrunesRemovedTasks bounds retained activity tails to mirrors still present in the snapshot.
func TestMirrorWatcherPrunesRemovedTasks(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	watcher := NewWatcher(home, filepath.Join(home, "bus"), 0)
	watcher.Poll()
	if len(watcher.mirrorCursors) != 1 {
		t.Fatalf("expected a baseline cursor, got %d", len(watcher.mirrorCursors))
	}
	if err := os.RemoveAll(dir); err != nil {
		t.Fatal(err)
	}
	watcher.Poll()
	if len(watcher.mirrorCursors) != 0 {
		t.Fatalf("removed mirror retains %d activity cursor(s)", len(watcher.mirrorCursors))
	}
	// Reappearance creates a fresh cursor and reports new activity normally.
	mirrorFixture(t, home)
	_, events := watcher.Poll()
	found := false
	for _, event := range events {
		found = found || event.Kind == "act" && strings.Contains(event.Text, "mirrored activity")
	}
	if !found || len(watcher.mirrorCursors) != 1 {
		t.Fatalf("reappearing mirror not tracked: %+v", events)
	}
}
