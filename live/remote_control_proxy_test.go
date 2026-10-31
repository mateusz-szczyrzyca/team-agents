package main

import (
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// TestRemoteControlProxyWebAnswers resolves a local remote-member proxy to the mirrored editable copy.
func TestRemoteControlProxyWebAnswers(t *testing.T) {
	home, ts := remoteControlServer(t)
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"),
		"ID=T_PROXY\nAGENT=worker@node\nSTATUS=waiting\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")
	var calls []string // agent subcommands invoked by the answer route
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		calls = append(calls, strings.Join(args, " "))
		return "Answers queued", nil
	})
	code, body := get(t, ts, "/questions/T_PROXY")
	if code != http.StatusOK || !strings.Contains(body, "Proceed?") {
		t.Fatalf("proxy answer form: status=%d body=%.200s", code, body)
	}
	code, body = post(t, ts, "/questions/T_PROXY", url.Values{"action": {"send"}, "c0": {"ok"}})
	if code != http.StatusOK || !strings.Contains(strings.ToLower(body), "queued") {
		t.Fatalf("proxy answer submission: status=%d body=%.200s", code, body)
	}
	if len(calls) != 1 || calls[0] != "_answers_saved remote@node 2" {
		t.Fatalf("proxy answer actions=%v", calls)
	}
	for _, target := range []struct {
		path     string // file whose ownership is checked
		answered bool   // only the editable copy receives the verdict
	}{
		{filepath.Join(home, "remote-questions", "node", "remote", "questions-2.md"), true},
		{filepath.Join(home, "bus", "remote", "node", "remote", "questions-2.md"), false},
	} {
		data, err := os.ReadFile(target.path)
		if err != nil || strings.Contains(string(data), "**ANSWERED**") != target.answered {
			t.Fatalf("proxy answer ownership %s: err=%v content=%s", target.path, err, data)
		}
	}
}

// TestRemoteControlProxyPanelKick uses kick semantics when stopping a local remote-member proxy in the panel.
func TestRemoteControlProxyPanelKick(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"),
		"ID=T_PROXY\nAGENT=worker@node\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")
	writeFile(t, filepath.Join(home, "bus", "remote", "node", "remote", "metadata.json"),
		`{"AGENT":"worker","STATUS":"running","RUN":"1","JOB_ID":"j-remote"}`)
	script := filepath.Join(t.TempDir(), "fake-agent")
	writeFile(t, script, "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$*\" > \"$0.args\"\necho 'Kick queued'\n")
	if err := os.Chmod(script, 0o700); err != nil { // #nosec G302 -- the private temporary fake agent needs owner execute permission
		t.Fatal(err)
	}
	m := newModel(config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: script, remoteControls: true})
	m.selID = "T_PROXY"
	next, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'s'}})
	confirmed := next.(model)
	if confirmed.mode != modeConfirmStop {
		t.Fatal("configured lead did not offer stop confirmation for local proxy")
	}
	_, cmd := confirmed.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	if cmd == nil {
		t.Fatal("confirmed proxy kick produced no command")
	}
	cmd()
	action := strings.TrimSpace(readFile(script + ".args"))
	if action != "kick T_PROXY live" {
		t.Fatalf("proxy confirmation action=%q, want %q", action, "kick T_PROXY live")
	}
}
