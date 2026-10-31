package main

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// remoteControlServer provides a configured lead with a waiting mirror and its editable copy.
func remoteControlServer(t *testing.T) (string, *httptest.Server) {
	t.Helper()
	home := fixture(t)
	dir := filepath.Join(home, "bus", "remote", "node", "remote")
	question := "# Questions\n\n## 1. Proceed?\na) yes\nb) no\n\n**Recommended:** a)\n\n**Verdict:**\n"
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker","STATUS":"waiting","RUN":"1","JOB_ID":"j-remote"}`)
	writeFile(t, filepath.Join(dir, "questions-2.md"), question)
	writeFile(t, filepath.Join(home, "remote-questions", "node", "remote", "questions-2.md"), question)
	s, err := newWebServer(webConfig{config: config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", remoteControls: true}})
	if err != nil {
		t.Fatal(err)
	}
	ts := httptest.NewServer(s.handler())
	t.Cleanup(ts.Close)
	return home, ts
}

// TestRemoteControlWebAnswers queues the editable copy without writing the read-only mirror.
func TestRemoteControlWebAnswers(t *testing.T) {
	home, ts := remoteControlServer(t)
	var calls []string // actions requested by the browser
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		calls = append(calls, strings.Join(args, " "))
		return "Answers queued", nil
	})
	code, body := get(t, ts, "/questions/remote@node")
	if code != http.StatusOK || !strings.Contains(body, "Proceed?") {
		t.Fatalf("lead answer form: status=%d body=%.200s", code, body)
	}
	code, body = post(t, ts, "/questions/remote@node", url.Values{"action": {"send"}, "round": {"2"}, "c0": {"ok"}})
	if code != http.StatusOK || !strings.Contains(strings.ToLower(body), "queued") {
		t.Fatalf("lead answer submission: status=%d body=%.200s", code, body)
	}
	if len(calls) != 1 || calls[0] != "_answers_saved remote@node 2" {
		t.Fatalf("answer actions=%v", calls)
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
			t.Fatalf("answer ownership %s: err=%v content=%s", target.path, err, data)
		}
	}
}

// TestRemoteControlWebKick queues the same qualified task action as the local dashboard.
func TestRemoteControlWebKick(t *testing.T) {
	_, ts := remoteControlServer(t)
	var action string // the sole requested kick
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		action = strings.Join(args, " ")
		return "Kick queued", nil
	})
	code, body := post(t, ts, "/task/remote@node/stop", nil)
	if code != http.StatusOK || action != "kick remote@node web" || !strings.Contains(body, "queued") {
		t.Fatalf("remote kick: status=%d action=%q body=%s", code, action, body)
	}
}

// TestRemoteControlQuestionsList exposes an answer link only for configured leads.
func TestRemoteControlQuestionsList(t *testing.T) {
	_, ts := remoteControlServer(t)
	code, body := get(t, ts, "/questions")
	if code != http.StatusOK || !strings.Contains(body, "/questions/remote@node") {
		t.Fatalf("remote questions list: status=%d body=%.400s", code, body)
	}
}
