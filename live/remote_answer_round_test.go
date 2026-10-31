package main

import (
	"net/http"
	"net/url"
	"path/filepath"
	"strings"
	"testing"
)

// TestRemoteAnswerFormCarriesRound binds browser submissions to the questions shown.
func TestRemoteAnswerFormCarriesRound(t *testing.T) {
	_, ts := remoteControlServer(t)
	code, body := get(t, ts, "/questions/remote@node")
	if code != http.StatusOK || !strings.Contains(body, `name="round" value="2"`) {
		t.Fatalf("answer form does not carry displayed round: status=%d", code)
	}
}

// TestRemoteAnswerRejectsUnboundRounds rejects stale, absent and malformed identities before any write or send.
func TestRemoteAnswerRejectsUnboundRounds(t *testing.T) {
	for _, route := range []string{"remote@node", "T_PROXY"} {
		for _, action := range []string{"draft", "send"} {
			for _, round := range []string{"2", "", "invalid", "0", "-1", "3", "4"} {
				t.Run(route+"/"+action+"/round="+round, func(t *testing.T) {
					home, ts := remoteControlServer(t)
					writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"),
						"ID=T_PROXY\nAGENT=worker@node\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=j-remote\n")
					code, _ := get(t, ts, "/questions/"+route)
					if code != http.StatusOK {
						t.Fatal("initial round-two form is unavailable")
					}
					question := "# Questions\n\n## 1. A different decision?\na) yes\nb) no\n\n**Recommended:** b)\n\n**Verdict:**\n"
					mirror := filepath.Join(home, "bus", "remote", "node", "remote", "questions-3.md")
					copyPath := filepath.Join(home, "remote-questions", "node", "remote", "questions-3.md")
					writeFile(t, mirror, question)
					writeFile(t, copyPath, question)
					oldPath := filepath.Join(home, "remote-questions", "node", "remote", "questions-2.md")
					old := readFile(oldPath)
					called := false // rejected submissions must never invoke the agent command
					stubAgent(t, func(...string) (string, *OpError[AgentCallCause]) {
						called = true
						return "Answers queued", nil
					})
					form := url.Values{"action": {action}, "c0": {"own"}, "t0": {"old answer"}}
					if round != "" {
						form.Set("round", round)
					}
					if round == "3" {
						// Duplicate identities are ambiguous even if one equals the current round.
						form["round"] = []string{"3", "2"}
					}
					code, _ = post(t, ts, "/questions/"+route, form)
					if code != http.StatusConflict || called || readFile(copyPath) != question || readFile(mirror) != question || readFile(oldPath) != old {
						t.Fatalf("unbound submission changed questions or sent: status=%d called=%v", code, called)
					}
				})
			}
		}
	}
}

// TestRemoteAnswerAcceptsCurrentRound retains draft and send behavior with the displayed identity.
func TestRemoteAnswerAcceptsCurrentRound(t *testing.T) {
	for _, action := range []string{"draft", "send"} {
		t.Run(action, func(t *testing.T) {
			home, ts := remoteControlServer(t)
			var called string // captures the exact round passed to the durable send command
			stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
				called = strings.Join(args, " ")
				return "Answers queued", nil
			})
			code, _ := post(t, ts, "/questions/remote@node", url.Values{"action": {action}, "round": {"2"}, "c0": {"ok"}})
			path := filepath.Join(home, "remote-questions", "node", "remote", "questions-2.md")
			if code != http.StatusOK || !strings.Contains(readFile(path), "**Verdict:** ok") {
				t.Fatalf("current round submission rejected: status=%d", code)
			}
			want := ""
			if action == "send" {
				want = "_answers_saved remote@node 2"
			}
			if called != want {
				t.Fatalf("command=%q want=%q", called, want)
			}
		})
	}
}
