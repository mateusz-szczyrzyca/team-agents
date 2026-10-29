package main

import (
	"html"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// TestMirrorPageStreamHandoffDuplicateBoundary verifies repeated lines at the stream handoff boundary do not lose events.
func TestMirrorPageStreamHandoffDuplicateBoundary(t *testing.T) {
	// Subtest 1 (inv-astra): A,B on page, replaced with B,C,B before SSE. Unseen C must be delivered.
	t.Run("repeated boundary with intermediate event", func(t *testing.T) {
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

		replaceMirrorActivity(t, path, b+c+b)
		br := openStream(t, ts, html.UnescapeString(match[1]))
		replaceMirrorActivity(t, path, b+c+b+mirrorActivityLine(4, "sentinel"))

		var events []string
		for {
			got := readEvent(t, br, "act")
			events = append(events, got)
			if strings.Contains(got, "unseen-c") || strings.Contains(got, "sentinel") {
				break
			}
		}
		if !strings.Contains(events[len(events)-1], "unseen-c") {
			t.Fatalf("handoff lost unseen event between repeated boundary lines; received: %v", events)
		}
	})

	// Subtest 2 (inv-kimi): [X] on page, replaced with [X,X] before SSE. Second X must be delivered.
	t.Run("appended identical line before SSE", func(t *testing.T) {
		home := fixture(t)
		dir := mirrorFixture(t, home)
		path := filepath.Join(dir, "activity.jsonl")
		x := mirrorActivityLine(1, "step-x")

		writeFile(t, path, x)
		_, ts := testServer(t, home)
		_, body := get(t, ts, "/task/T1@node-a")
		match := regexp.MustCompile(`sse-connect="([^"]+)"`).FindStringSubmatch(body)
		if len(match) != 2 {
			t.Fatal("activity page has no stream URL")
		}

		replaceMirrorActivity(t, path, x+x)
		br := openStream(t, ts, html.UnescapeString(match[1]))
		replaceMirrorActivity(t, path, x+x+mirrorActivityLine(2, "sentinel"))

		var events []string
		for {
			got := readEvent(t, br, "act")
			events = append(events, got)
			if strings.Contains(got, "step-x") || strings.Contains(got, "sentinel") {
				break
			}
		}
		if !strings.Contains(events[len(events)-1], "step-x") {
			t.Fatalf("handoff lost appended identical event; received: %v", events)
		}
	})

	// Subtest 3 (codex-1): B,A,B on page, replaced with B,C,B before SSE. Unseen C must be delivered.
	t.Run("boundary repeated with preceding identical line", func(t *testing.T) {
		home := fixture(t)
		dir := mirrorFixture(t, home)
		path := filepath.Join(dir, "activity.jsonl")
		b := mirrorActivityLine(1, "repeated-b")
		a := mirrorActivityLine(2, "past-a")
		c := mirrorActivityLine(3, "unseen-c")

		writeFile(t, path, b+a+b)
		_, ts := testServer(t, home)
		_, body := get(t, ts, "/task/T1@node-a")
		match := regexp.MustCompile(`sse-connect="([^"]+)"`).FindStringSubmatch(body)
		if len(match) != 2 {
			t.Fatal("activity page has no stream URL")
		}

		replaceMirrorActivity(t, path, b+c+b)
		br := openStream(t, ts, html.UnescapeString(match[1]))
		replaceMirrorActivity(t, path, b+c+b+mirrorActivityLine(4, "sentinel"))

		var events []string
		for {
			got := readEvent(t, br, "act")
			events = append(events, got)
			if strings.Contains(got, "unseen-c") || strings.Contains(got, "sentinel") {
				break
			}
		}
		if !strings.Contains(events[len(events)-1], "unseen-c") {
			t.Fatalf("handoff lost unseen event between repeated boundary lines; received: %v", events)
		}
	})
}

// TestMirrorLogsComponentlessRemoteFiltered verifies componentless remote records do not bypass src filtering.
func TestMirrorLogsComponentlessRemoteFiltered(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	remDir := filepath.Join(home, "bus", "remote", "node-a")
	writeFile(t, filepath.Join(remDir, "logs.jsonl"),
		`{"ts":"2026-10-23T06:02:00Z","level":"info","component":"agent","event":"agent_step","task":"T-rem-1"}`+"\n"+
			`{"ts":"2026-10-23T06:03:00Z","level":"info","component":"agentlive","event":"live_step","task":"T-rem-1"}`+"\n"+
			`{"ts":"2026-10-23T06:04:00Z","level":"info","event":"componentless_step","task":"T-rem-1"}`+"\n")

	_, ts := testServer(t, home)

	_, bodyLive := get(t, ts, "/logs?src=agentlive")
	if strings.Contains(bodyLive, "componentless_step") {
		t.Errorf("/logs?src=agentlive included componentless remote record: %s", bodyLive)
	}
	if !strings.Contains(bodyLive, "live_step") {
		t.Errorf("/logs?src=agentlive missing live_step: %s", bodyLive)
	}

	_, bodyAgent := get(t, ts, "/logs?src=agent")
	if strings.Contains(bodyAgent, "componentless_step") {
		t.Errorf("/logs?src=agent included componentless remote record: %s", bodyAgent)
	}
	if !strings.Contains(bodyAgent, "agent_step") {
		t.Errorf("/logs?src=agent missing agent_step: %s", bodyAgent)
	}
}
