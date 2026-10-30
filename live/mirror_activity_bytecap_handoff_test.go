package main

import (
	"encoding/json"
	"fmt"
	"html"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// TestMirrorPageStreamHandoffRolledOutBoundary verifies that rolled out boundaries or duplicate lines at SSE handoff do not lose unseen events.
func TestMirrorPageStreamHandoffRolledOutBoundary(t *testing.T) {
	// Subtest 1 (inv-kimi / review61): A,B on page replaced with C,B before SSE. Unseen C must be delivered.
	t.Run("intermediate unseen event before duplicate boundary", func(t *testing.T) {
		home := fixture(t)
		dir := mirrorFixture(t, home)
		path := filepath.Join(dir, "activity.jsonl")
		a := mirrorActivityLine(1, "past-a")
		b := mirrorActivityLine(2, "repeated-b")
		c := mirrorActivityLine(3, "unseen-c")

		writeFile(t, path, a+b)
		_, ts := testServer(t, home)
		_, body := get(t, ts, "/task/T1@node-a")
		match := regexp.MustCompile(`sse-connect="([^"]+)"`).FindStringSubmatch(body)
		if len(match) != 2 {
			t.Fatal("activity page has no stream URL")
		}

		replaceMirrorActivity(t, path, c+b)
		br := openStream(t, ts, html.UnescapeString(match[1]))
		replaceMirrorActivity(t, path, c+b+mirrorActivityLine(4, "sentinel"))

		var events []string
		for {
			got := readEvent(t, br, "act")
			events = append(events, got)
			if strings.Contains(got, "unseen-c") || strings.Contains(got, "sentinel") {
				break
			}
		}
		if !strings.Contains(events[len(events)-1], "unseen-c") {
			t.Fatalf("handoff lost unseen event before duplicate boundary; received: %v", events)
		}
	})

	// Subtest 2 (inv-astra / review60 M1): A,B on page, then 256 KiB tail ending in duplicate B. Unseen events must be delivered.
	t.Run("rolled out original boundary in byte-capped tail", func(t *testing.T) {
		home := fixture(t)
		dir := mirrorFixture(t, home)
		path := filepath.Join(dir, "activity.jsonl")
		a := mirrorActivityLine(1, "past-a")
		b := mirrorActivityLine(2, "boundary-b")

		writeFile(t, path, a+b)
		_, ts := testServer(t, home)
		_, body := get(t, ts, "/task/T1@node-a")
		match := regexp.MustCompile(`sse-connect="([^"]+)"`).FindStringSubmatch(body)
		if len(match) != 2 {
			t.Fatal("activity page has no stream URL")
		}

		var sb strings.Builder
		for i := 1; i <= 440; i++ {
			fmt.Fprintf(&sb, "{\"ts\":\"10:01:%02d\",\"k\":\"text\",\"t\":%q}\n", i%60, fmt.Sprintf("unseen-%03d-%s", i, strings.Repeat("x", 550)))
		}
		sb.WriteString(b)
		fullTail := sb.String()
		if len(fullTail) > 262144 {
			fullTail = fullTail[len(fullTail)-262144:]
		}

		replaceMirrorActivity(t, path, fullTail)
		br := openStream(t, ts, html.UnescapeString(match[1]))
		replaceMirrorActivity(t, path, fullTail+mirrorActivityLine(5, "sentinel"))

		foundUnseen := false
		for i := 0; i < 20; i++ {
			got := readEvent(t, br, "act")
			if strings.Contains(got, "unseen-") {
				foundUnseen = true
				break
			}
			if strings.Contains(got, "sentinel") {
				break
			}
		}
		if !foundUnseen {
			t.Fatal("handoff consumed entire tail without delivering unseen records")
		}
	})
}

// TestMirrorActivityByteCapLeadingFragment verifies that byte-capped tails starting with incomplete JSON fragments do not replay on appends.
func TestMirrorActivityByteCapLeadingFragment(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	path := filepath.Join(dir, "activity.jsonl")

	var sb strings.Builder
	for i := 1; i <= 500; i++ {
		fmt.Fprintf(&sb, "{\"ts\":\"10:00:%02d\",\"k\":\"text\",\"t\":%q}\n", i%60, fmt.Sprintf("entry-%03d-%s", i, strings.Repeat("y", 560)))
	}
	initial500 := sb.String()
	if len(initial500) < 262144 {
		t.Fatalf("test data too small: %d bytes", len(initial500))
	}
	capped500 := initial500[len(initial500)-262144:]
	if len(capped500) != 262144 {
		t.Fatalf("expected capped length 262144, got %d", len(capped500))
	}
	firstLine := strings.SplitN(capped500, "\n", 2)[0] + "\n"
	var dummy Activity
	if json.Unmarshal([]byte(firstLine), &dummy) == nil {
		t.Fatalf("expected leading fragment to be invalid JSON, got valid: %s", firstLine)
	}
	writeFile(t, path, capped500)

	cursor := &mirrorActivityCursor{}
	initialActs := cursor.read(path)
	if len(initialActs) == 0 {
		t.Fatal("initial read returned no activities")
	}

	appended := initial500 + mirrorActivityLine(59, "entry-501-target")
	capped501 := appended[len(appended)-262144:]
	replaceMirrorActivity(t, path, capped501)

	newActs := cursor.read(path)
	if len(newActs) != 1 {
		t.Fatalf("expected exactly 1 new activity after append, got %d (unseen history replayed)", len(newActs))
	}
	if newActs[0].Text != "entry-501-target" {
		t.Fatalf("expected entry-501-target, got %q", newActs[0].Text)
	}
}

// TestMirrorActivityByteCapLegacyOffset verifies that baseline raw-byte offset accounting handles leading byte-cap fragments correctly.
func TestMirrorActivityByteCapLegacyOffset(t *testing.T) {
	home := fixture(t)
	dir := mirrorFixture(t, home)
	path := filepath.Join(dir, "activity.jsonl")

	line1 := mirrorActivityLine(1, "first")
	line2 := mirrorActivityLine(2, "second")
	fragment := line1[10:] // incomplete JSON fragment
	writeFile(t, path, fragment+line2)

	cursor := &mirrorActivityCursor{}
	cursor.baseline(path, int64(len(fragment)))

	acts := cursor.read(path)
	if len(acts) != 1 || acts[0].Text != "second" {
		t.Fatalf("baseline offset failed to advance past leading fragment: %+v", acts)
	}
}
