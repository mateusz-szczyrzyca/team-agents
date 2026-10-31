package main

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// TestRemoteControlPanelAnswers enables the existing answer command only for a configured lead.
func TestRemoteControlPanelAnswers(t *testing.T) {
	m, _ := historyMirrorModel(t, "waiting")
	m.cfg.remoteControls = true
	_, cmd := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'a'}})
	if cmd == nil {
		t.Fatal("configured lead did not offer its qualified answer command")
	}
}

// TestRemoteControlPanelKick uses kick semantics for a remote run after explicit confirmation.
func TestRemoteControlPanelKick(t *testing.T) {
	m, _ := historyMirrorModel(t, "running")
	m.cfg.remoteControls = true
	script := filepath.Join(t.TempDir(), "fake-agent")
	writeFile(t, script, "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$*\" > \"$0.args\"\necho 'Kick queued'\n")
	if err := os.Chmod(script, 0o700); err != nil { // #nosec G302 -- the private temporary fake agent needs owner execute permission
		t.Fatal(err)
	}
	m.cfg.agentCmd = script
	next, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'s'}})
	confirmed := next.(model)
	if confirmed.mode != modeConfirmStop {
		t.Fatal("configured lead did not offer remote stop confirmation")
	}
	_, cmd := confirmed.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	if cmd == nil {
		t.Fatal("confirmed remote kick produced no command")
	}
	cmd()
	action := strings.TrimSpace(readFile(script + ".args"))
	if action != "kick remote@node live" {
		t.Fatalf("remote confirmation action=%q", action)
	}
}

// TestRemoteControlDashboardButton advertises the qualified stop route for a running mirror.
func TestRemoteControlDashboardButton(t *testing.T) {
	home, ts := remoteControlServer(t)
	writeFile(t, filepath.Join(home, "bus", "remote", "node", "remote", "metadata.json"), `{"AGENT":"worker","STATUS":"running","RUN":"1","JOB_ID":"j-remote"}`)
	code, body := get(t, ts, "/")
	if code != http.StatusOK || !strings.Contains(body, `hx-post="/task/remote@node/stop"`) {
		t.Fatalf("configured dashboard did not advertise remote kick: status=%d", code)
	}
}
