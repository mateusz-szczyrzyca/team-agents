package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// hubViewFixtureJSON supplies metadata from two hosts with the same raw task ID and agent base name.
func hubViewFixtureJSON(t *testing.T) []byte {
	t.Helper()
	at := time.Date(2026, 10, 22, 12, 0, 0, 0, time.UTC)
	view := hubViewData{Version: 1, At: at.Format(time.RFC3339Nano), Instances: []hubViewInstance{
		{Name: "mac", Active: true, ExpiresAt: at.Add(time.Minute).Format(time.RFC3339), OS: "darwin", Arch: "arm64", Tasks: map[string]hubViewTask{
			"shared": {ID: "shared", Agent: "codex", Repo: "example-app", Branch: "agents/mac", Status: "running"}}},
		{Name: "linux", Active: false, ExpiresAt: at.Add(-time.Minute).Format(time.RFC3339), OS: "linux", Arch: "amd64", Tasks: map[string]hubViewTask{
			"shared": {ID: "shared", Agent: "codex", Repo: "example-app", Branch: "agents/linux", Status: "sleeping"}}},
	}}
	data, err := json.Marshal(view)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

// TestHubViewTasksStayQualifiedAndMetadataOnly prevents ID collisions and trust in remote paths.
func TestHubViewTasksStayQualifiedAndMetadataOnly(t *testing.T) {
	snapshot, err := decodeHubView(hubViewFixtureJSON(t))
	if err != nil || len(snapshot.Tasks) != 2 || len(snapshot.All) != 2 {
		t.Fatalf("aggregate task projection: %+v %v", snapshot, err)
	}
	mac, linux := snapshot.All["shared@mac"], snapshot.All["shared@linux"]
	if mac == nil || linux == nil || mac.Agent != "codex@mac" || linux.Status != statusUnreachable || !linux.Active() {
		t.Fatalf("qualified or expired task identity: %+v %+v", mac, linux)
	}
	for _, task := range snapshot.Tasks {
		if !task.Remote || task.Dir != "" || task.Worktree != "" {
			t.Fatal("hub summary received local filesystem authority")
		}
	}
}

// TestHubTaskReadersAndAckDoNotUseRemotePaths verifies guards even if a caller supplies a local-looking path.
func TestHubTaskReadersAndAckDoNotUseRemotePaths(t *testing.T) {
	dir := t.TempDir()
	for _, name := range []string{"final.md", "activity.jsonl", "questions-0.md", "task.md"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("private local contents"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	task := &Task{ID: "shared@mac", Agent: "codex@mac", Instance: "mac", Remote: true, Status: "done", Dir: dir, Worktree: dir}
	if task.finalSummary() != "" || len(NewWatcher(dir, dir, 0).newActivity(task, true)) != 0 {
		t.Fatal("a remote summary read local task files")
	}
	model := model{snap: &Snapshot{All: map[string]*Task{task.ID: task}, Tasks: []*Task{task}, Agents: map[string]*Presence{}}, selID: task.ID}
	for tab := 0; tab < 7; tab++ {
		model.tab = tab
		if strings.Contains(model.detail(task), "private local contents") {
			t.Fatalf("remote detail tab %d opened local data", tab)
		}
	}
	for _, key := range []rune{'x', 'X'} {
		model.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{key}})
		if _, err := os.Stat(filepath.Join(dir, "acked")); !os.IsNotExist(err) {
			t.Fatalf("remote acknowledgment touched local files: %v", err)
		}
	}
}

// TestHubViewOutageRetainsLastSnapshotWithoutReadingLocalBus covers the source actually selected by the panel.
func TestHubViewOutageRetainsLastSnapshotWithoutReadingLocalBus(t *testing.T) {
	home := t.TempDir()
	if err := os.WriteFile(filepath.Join(home, "view.json"), hubViewFixtureJSON(t), 0o600); err != nil {
		t.Fatal(err)
	}
	helper := filepath.Join(home, "snapshot-stub")
	script := "#!/bin/sh\nset -eu\nif [ -f \"$AGENTS_HOME/down\" ]; then exit 11; fi\ncat \"$AGENTS_HOME/view.json\"\n"
	if err := os.WriteFile(helper, []byte(script), 0o700); err != nil { //nolint:gosec // G306: private executable test stub under t.TempDir, never a real agentbus or agent CLI
		t.Fatal(err)
	}
	watcher := configuredWatcher(config{home: home, busDir: filepath.Join(home, "must-not-read-bus"), snapshotCmd: helper})
	first, _ := watcher.Poll()
	if len(first.Tasks) != 2 || first.HubError != "" {
		t.Fatalf("initial hub view: %+v", first)
	}
	if err := os.WriteFile(filepath.Join(home, "down"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	stale, _ := watcher.Poll()
	if len(stale.Tasks) != 2 || stale.HubError == "" || first.HubError != "" {
		t.Fatalf("outage discarded or mutated the good snapshot: %+v", stale)
	}
	if _, err := os.Stat(filepath.Join(home, "must-not-read-bus")); !os.IsNotExist(err) {
		t.Fatalf("hub outage created a second local bus: %v", err)
	}
	if err := os.Remove(filepath.Join(home, "down")); err != nil {
		t.Fatal(err)
	}
	recovered, _ := watcher.Poll()
	if len(recovered.Tasks) != 2 || recovered.HubError != "" {
		t.Fatalf("hub view did not recover: %+v", recovered)
	}
}

// TestHubViewRejectsInvalidDisplaySchema prevents malformed metadata from constructing local task references.
func TestHubViewRejectsInvalidDisplaySchema(t *testing.T) {
	for _, body := range []string{`invalid`, `{"version":2}`, `{"version":1,"at":"invalid"}`,
		`{"version":1,"at":"2026-10-22T12:00:00Z","instances":[{"name":"mac","expires_at":"2026-10-22T12:01:00Z","tasks":{"../escape":{"task_id":"../escape"}}}]}`} {
		if _, err := decodeHubView([]byte(body)); err == nil {
			t.Fatalf("malformed view was accepted: %s", body)
		}
	}
}
