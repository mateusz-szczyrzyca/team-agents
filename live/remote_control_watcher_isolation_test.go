package main

import (
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestFrontendStreamDashboardWatcherIsolation verifies that streaming dashboard keeps watcher baseline
// raw and does not emit spurious status transition events across ticks.
func TestFrontendStreamDashboardWatcherIsolation(t *testing.T) {
	home := fixture(t)
	if err := os.RemoveAll(filepath.Join(home, "tasks")); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(home, "bus", "remote", "node", "remote")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker","STATUS":"running","RUN":"1","JOB_ID":"j-remote"}`)
	writeFile(t, filepath.Join(dir, "questions-1.md"), "# Questions\n\n## 1. Proceed?\na) yes\n**Verdict:**\n")
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), `{"Version":1,"Jobs":[{"Request":{"ID":"j-remote","Repository":"repo","Team":"team"},"Node":"node","TaskID":"remote","State":"waiting"}]}`)

	s, err := newWebServer(webConfig{
		config: config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", remoteControls: true},
		poll:   15 * time.Millisecond,
	})
	if err != nil {
		t.Fatal(err)
	}
	ts := httptest.NewServer(s.handler())
	t.Cleanup(ts.Close)

	br := openStream(t, ts, "/events/dashboard")
	table := readEvent(t, br, "table")
	if !strings.Contains(table, `class="st st-waiting"`) && !strings.Contains(table, "waiting for you") {
		t.Fatalf("streamed dashboard table event dropped remote waiting override: %q", table)
	}

	heads := 0
	for heads < 3 {
		line, err := br.ReadString('\n')
		if err != nil {
			t.Fatalf("stream ended: %v", err)
		}
		if strings.Contains(line, "waiting → running") {
			t.Fatalf("spurious status flicker emitted: %q", line)
		}
		if strings.HasPrefix(line, "event: head") {
			heads++
		}
	}
}
