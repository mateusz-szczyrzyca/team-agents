package main

import (
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// historyMirrorModel supplies a selected remote task directly so Round is not lost by the mirror whitelist.
func historyMirrorModel(t *testing.T, status string) (model, *Task) {
	t.Helper()
	home := fixture(t)
	m := newModel(config{home: home, busDir: filepath.Join(home, "bus")})
	task := &Task{ID: "remote@node", Node: "node", Agent: "worker", Dir: t.TempDir(), Status: status, Round: 2, RunStart: time.Now()}
	m.snap = &Snapshot{All: map[string]*Task{task.ID: task}, Tasks: []*Task{task}, Agents: map[string]*Presence{}}
	m.selID = task.ID
	return m, task
}

// TestMirrorHistoryTerminalAnswer checks the remote guard with an actual positive question round.
func TestMirrorHistoryTerminalAnswer(t *testing.T) {
	m, task := historyMirrorModel(t, "waiting")
	if task.Round <= 0 {
		t.Fatal("guard regression requires positive Round")
	}
	next, cmd := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'a'}})
	if cmd != nil || next.(model).flash != "mirrored task cannot be answered from lead" {
		t.Fatal("remote answer scheduled an execution or missed its guard")
	}
}

// TestMirrorHistoryTerminalStop checks the guard before confirmation or command execution.
func TestMirrorHistoryTerminalStop(t *testing.T) {
	m, _ := historyMirrorModel(t, "running")
	next, cmd := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'s'}})
	if cmd != nil || next.(model).mode == modeConfirmStop {
		t.Fatal("remote stop entered confirmation or scheduled execution")
	}
}

// TestMirrorHistoryTerminalLabels checks remote process liveness is not inferred from a lead PID.
func TestMirrorHistoryTerminalLabels(t *testing.T) {
	m, task := historyMirrorModel(t, "running")
	task.NodeOffline = true
	row := m.row(task, 160)
	if strings.Contains(row, "process is dead") || !strings.Contains(row, "node offline") {
		t.Fatalf("remote row lost last-known status: %s", row)
	}
}

// TestMirrorHistoryWebAnswer checks the typed guard before loading or writing a valid remote question.
func TestMirrorHistoryWebAnswer(t *testing.T) {
	m, task := historyMirrorModel(t, "waiting")
	writeFile(t, filepath.Join(task.Dir, "questions-2.md"), "# Questions\n\n## 1. Proceed?\na) yes\nb) no\n\n**Recommended:** a)\n\n**Verdict:**\n")
	server, _ := testServer(t, m.cfg.home)
	_, _, _, err := server.loadAnswer(m.snap, task.ID)
	if err == nil || err.Cause != AnswerNotWaiting {
		t.Fatalf("remote Round=2 answer guard: %v", err)
	}
}

// TestMirrorHistoryWebStop checks the HTTP guard rejects mirrored stop requests.
func TestMirrorHistoryWebStop(t *testing.T) {
	home := fixture(t)
	dir := filepath.Join(home, "bus", "remote", "node", "remote")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker","STATUS":"running"}`)
	_, ts := testServer(t, home)
	code, _ := post(t, ts, "/task/remote@node/stop", nil)
	if code != 403 {
		t.Fatalf("remote stop status %d, want 403", code)
	}
}

// TestMirrorHistoryQuestionFiles checks that mirrored rounds are inferred rather than taken from ROUND.
func TestMirrorHistoryQuestionFiles(t *testing.T) {
	home := fixture(t)
	dir := filepath.Join(home, "bus", "remote", "node", "remote")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker","STATUS":"waiting"}`)
	writeFile(t, filepath.Join(dir, "questions-1.md"), "first mirrored question")
	writeFile(t, filepath.Join(dir, "questions-2.md"), "second mirrored question")
	_, ts := testServer(t, home)
	code, body := get(t, ts, "/task/remote@node?tab=questions")
	if code != 200 || !strings.Contains(body, "first mirrored question") || !strings.Contains(body, "second mirrored question") || strings.Contains(body, ">Answer<") {
		t.Fatal("mirror questions were omitted or offer a lead answer")
	}
}

// TestMirrorHistoryDirectoryDiagnostic reports a non-ENOENT directory failure from the logs handler.
func TestMirrorHistoryDirectoryDiagnostic(t *testing.T) {
	home := fixture(t)
	server, _ := testServer(t, home)
	writeFile(t, filepath.Join(home, "bus", "remote"), "not a directory")
	logs := captureLogs(t)
	recorder := httptest.NewRecorder()
	server.viewLogs(recorder, httptest.NewRequest("GET", "/logs", nil))
	if recorder.Code != 200 || !strings.Contains(logs.String(), `"event":"tasks_list_failed"`) || !strings.Contains(logs.String(), `"class":"Storage"`) {
		t.Fatalf("directory read failure was not reported: status=%d logs=%s", recorder.Code, logs.String())
	}
}
