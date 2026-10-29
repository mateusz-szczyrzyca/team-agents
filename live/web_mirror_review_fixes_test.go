package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// TestMirrorExactTaskLookupM3 verifies exact task ID lookup when qualified mirror keys share prefixes.
func TestMirrorExactTaskLookupM3(t *testing.T) {
	home := fixture(t)
	mirrorFixture(t, home)
	writeFile(t, filepath.Join(home, "bus", "remote", "node-b", "T1", "metadata.json"), `{"AGENT":"worker-1","STATUS":"done"}`)
	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	for i := 0; i < 1000; i++ {
		task := findTask(snap, "T1")
		if task == nil || task.ID != "T1" {
			t.Fatalf("exact T1 lookup failed on attempt %d: got %+v", i, task)
		}
	}
}

// TestMirrorPromptTabBoundsM4 verifies prompt rendering does not loop on large RUN metadata.
func TestMirrorPromptTabBoundsM4(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	dir := filepath.Join(home, "bus", "remote", "node-a", "T-rem-1")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"running","RUN":"10000"}`)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/task/T-rem-1@node-a?tab=prompt")
	if n := strings.Count(body, `<details class="file"`); n != 1 {
		t.Errorf("one mirrored prompt generated %d file panels (length %d bytes)", n, len(body))
	}
}

// TestMirrorQuestionNumericSortM2 verifies descending numeric order of mirrored question rounds.
func TestMirrorQuestionNumericSortM2(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	dir := filepath.Join(home, "bus", "remote", "node-a", "T-rem-1")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"waiting"}`)
	writeFile(t, filepath.Join(dir, "questions-9.md"), "round nine")
	writeFile(t, filepath.Join(dir, "questions-10.md"), "round ten")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/task/T-rem-1@node-a?tab=questions")
	if !before(body, "questions-10.md (round 10)", "questions-9.md (round 9)") {
		t.Error("round 9 sorted above round 10")
	}
}

// TestMirrorAgentIsolationM2 verifies mirrored tasks do not leak into local agent views or bus names.
func TestMirrorAgentIsolationM2(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	writeFile(t, filepath.Join(home, "bus", "remote", "node-a", "T-rem-1", "metadata.json"), `{"AGENT":"solo","STATUS":"waiting"}`)
	stubLocalAgents(t)
	s, ts := testServer(t, home)
	_, body := get(t, ts, "/agents/solo")
	if strings.Contains(body, `href="/task/T-rem-1@node-a"`) {
		t.Error("local /agents/solo includes remote node-a task")
	}
	task := s.snapshot().All["T-rem-1@node-a"]
	if task.BusName() != "solo@node-a" {
		t.Errorf("task.BusName() = %q, want solo@node-a", task.BusName())
	}
}

// TestMirrorAcknowledgementM1 verifies acknowledgement markers are read and hide mirrored tasks.
func TestMirrorAcknowledgementM1(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	writeFile(t, filepath.Join(home, "bus", "remote", "node-a", "T-rem-1", "metadata.json"), `{"AGENT":"worker-1","STATUS":"done"}`)
	cfg := config{home: home, busDir: filepath.Join(home, "bus")}
	m := newModel(cfg)
	m.selID = "T-rem-1@node-a"
	m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'x'}})
	snap := loadSnapshot(home, cfg.busDir)
	if !snap.All[m.selID].Acked {
		t.Error("x writes acked but reloaded mirror remains unacknowledged")
	}
	for _, task := range snap.Tasks {
		if task.ID == m.selID {
			t.Error("acknowledged mirror remains in terminal task list")
		}
	}
}

// TestMirrorLogsSrcFilterMinor1 verifies /logs?src= filters remote records by component.
func TestMirrorLogsSrcFilterMinor1(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	remDir := filepath.Join(home, "bus", "remote", "node-a")
	writeFile(t, filepath.Join(remDir, "logs.jsonl"),
		`{"ts":"2026-10-23T06:02:00Z","level":"info","component":"agent","event":"agent_step","task":"T-rem-1"}`+"\n"+
			`{"ts":"2026-10-23T06:03:00Z","level":"info","component":"agentlive","event":"live_step","task":"T-rem-1"}`+"\n")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/logs?src=agentlive")
	if strings.Contains(body, "agent_step") {
		t.Errorf("/logs?src=agentlive included component 'agent': %s", body)
	}
	if !strings.Contains(body, "live_step") {
		t.Errorf("/logs?src=agentlive missing component 'agentlive': %s", body)
	}
}

// TestMirrorQuestionsLeadAnswerMinor4 verifies /questions does not show broken answer link for mirrored tasks.
func TestMirrorQuestionsLeadAnswerMinor4(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	metaFile := filepath.Join(home, "bus", "remote", "node-a", "T-rem-1", "metadata.json")
	writeFile(t, metaFile, `{"STATUS":"waiting","AGENT":"worker-1","TYPE":"worker"}`)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/questions")
	if strings.Contains(body, "/questions/T-rem-1@node-a") {
		t.Errorf("/questions should not render answer link to mirrored task: %s", body)
	}
}

// TestMirrorTerminalKeyGuardsM7 verifies terminal key actions 'a', 's', and 'm' on mirrors.
func TestMirrorTerminalKeyGuardsM7(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	dir := filepath.Join(home, "bus", "remote", "node-a", "T-rem-1")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"waiting","ROUND":"2"}`)
	writeFile(t, filepath.Join(dir, "questions-2.md"), "questions")
	cfg := config{home: home, busDir: filepath.Join(home, "bus")}
	m := newModel(cfg)
	snap := loadSnapshot(home, cfg.busDir)
	m.snap = snap
	m.selID = "T-rem-1@node-a"
	// Key 'a' with Round > 0 on remote task should not open answer prompt
	m2, cmd := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'a'}})
	m = m2.(model)
	if cmd != nil {
		t.Error("key 'a' on mirrored task scheduled command execution")
	}
	if m.flash != "mirrored task cannot be answered from lead" {
		t.Errorf("key 'a' flash = %q, want 'mirrored task cannot be answered from lead'", m.flash)
	}
	// Key 's' on remote task should not open stop confirmation
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"running"}`)
	m.snap = loadSnapshot(home, cfg.busDir)
	m2, _ = m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'s'}})
	m = m2.(model)
	if m.mode == modeConfirmStop {
		t.Error("key 's' on mirrored task entered modeConfirmStop")
	}
	if m.flash != "mirrored task cannot be stopped from lead" {
		t.Errorf("key 's' flash = %q, want 'mirrored task cannot be stopped from lead'", m.flash)
	}
	// Key 'm' should target BusName with qualified agent@node
	m2, _ = m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'m'}})
	m = m2.(model)
	if m.target != "worker-1@node-a" {
		t.Errorf("key 'm' target = %q, want 'worker-1@node-a'", m.target)
	}
}

// TestMirrorDirectoriesReadErrorM8 verifies mirrorDirectories logs errors when directory cannot be listed.
func TestMirrorDirectoriesReadErrorM8(t *testing.T) {
	home := fixture(t)
	unreadable := filepath.Join(home, "unreadable")
	if err := os.Mkdir(unreadable, 0000); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := os.Remove(unreadable); err != nil {
			t.Errorf("remove empty unreadable fixture: %v", err)
		}
	}()
	dirs := mirrorDirectories(unreadable)
	if dirs != nil {
		t.Errorf("mirrorDirectories on unreadable directory: want nil, got %v", dirs)
	}
}
