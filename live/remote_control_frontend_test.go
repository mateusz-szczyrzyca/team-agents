package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// TestFrontendRemoteWaitingBadge checks that answerable remote mirrors appear in the waiting counters and badge,
// and that local remote-member proxies are properly deduplicated.
func TestFrontendRemoteWaitingBadge(t *testing.T) {
	home, ts := remoteControlServer(t)
	if err := os.RemoveAll(filepath.Join(home, "tasks")); err != nil {
		t.Fatal(err)
	}
	// Add a local running proxy referencing the same remote job to verify deduplication.
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"), "ID=T_PROXY\nAGENT=worker@node\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")

	code, body := get(t, ts, "/questions")
	if code != http.StatusOK || !strings.Contains(body, "/questions/remote@node") {
		t.Fatal("fixture did not expose remote question")
	}
	// The badge in the navigation bar should indicate exactly 1 task waiting for you.
	if !strings.Contains(body, "? 1 waiting for you") {
		t.Fatalf("answerable remote task is absent or incorrectly counted in waiting badges: %q", body)
	}
}

// TestFrontendProxyPanelWaiting checks that a local running proxy whose mirrored task is waiting can be answered in the terminal.
func TestFrontendProxyPanelWaiting(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"), "ID=T_PROXY\nAGENT=worker@node\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")
	writeFile(t, filepath.Join(home, "bus", "remote", "node", "remote", "metadata.json"), `{"AGENT":"worker","STATUS":"waiting","RUN":"1","JOB_ID":"j-remote"}`)
	writeFile(t, filepath.Join(home, "bus", "remote", "node", "remote", "questions-1.md"), "# Questions\n\n## 1. Q\n**Verdict:**\n")
	m := newModel(config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", remoteControls: true})
	m.selID = "T_PROXY"
	_, cmd := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'a'}})
	if cmd == nil {
		t.Fatal("waiting remote member cannot be answered from its retained running proxy")
	}
	m.tab = 2
	detail := m.detail(m.selected())
	if !strings.Contains(detail, "WAITING FOR ANSWERS") {
		t.Fatal("questions tab does not show waiting status for proxy")
	}
}

// TestFrontendProxyTaskAnswerLink checks that the task page of a running proxy links to answers when its mirror is waiting.
func TestFrontendProxyTaskAnswerLink(t *testing.T) {
	home, ts := remoteControlServer(t)
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"), "ID=T_PROXY\nAGENT=worker@node\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")
	code, body := get(t, ts, "/task/T_PROXY")
	if code != http.StatusOK {
		t.Fatalf("task status %d", code)
	}
	if !strings.Contains(body, `href="/questions/T_PROXY"`) && !strings.Contains(body, `href="/questions/remote@node"`) {
		t.Fatal("retained proxy task page has no answer link to its waiting mirror")
	}
}

// TestFrontendStreamDashboardRemoteControls checks that SSE dashboard streaming retains remote controls and waiting override across ticks.
func TestFrontendStreamDashboardRemoteControls(t *testing.T) {
	home := fixture(t)
	if err := os.RemoveAll(filepath.Join(home, "tasks")); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(home, "bus", "remote", "node", "remote")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker","STATUS":"running","RUN":"1","JOB_ID":"j-remote"}`)
	s, err := newWebServer(webConfig{
		config: config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", remoteControls: true},
		poll:   20 * time.Millisecond,
	})
	if err != nil {
		t.Fatal(err)
	}
	ts := httptest.NewServer(s.handler())
	t.Cleanup(ts.Close)

	br := openStream(t, ts, "/events/dashboard")
	// Phase 1: Running remote task must offer a stop button on the live SSE table.
	table := readEvent(t, br, "table")
	if !strings.Contains(table, `hx-post="/task/remote@node/stop"`) {
		t.Fatalf("streamed dashboard table event dropped remote stop button: %q", table)
	}

	// Phase 2: Metadata stays running on disk, but lead jobs registry transitions to waiting.
	writeFile(t, filepath.Join(dir, "questions-1.md"), "# Questions\n\n## 1. Proceed?\na) yes\n**Verdict:**\n")
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), `{"Version":1,"Jobs":[{"Request":{"ID":"j-remote","Repository":"repo","Team":"team"},"Node":"node","TaskID":"remote","State":"waiting"}]}`)

	table = readEvent(t, br, "table")
	if !strings.Contains(table, `class="st st-waiting"`) && !strings.Contains(table, "waiting for you") {
		t.Fatalf("streamed dashboard table event dropped remote waiting override: %q", table)
	}
	head := readEvent(t, br, "head")
	if !strings.Contains(head, "waiting for you: 1") {
		t.Fatalf("streamed dashboard header dropped waiting counter: %q", head)
	}
}
