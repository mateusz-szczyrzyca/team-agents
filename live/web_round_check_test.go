package main

import (
	"net/http"
	"strings"
	"testing"
)

// TestRoundCardPage: a team's page links its round card, whose page shows what `agent round-check <team>` prints —
// its board, its members' work not in the round's PR, the files two branches changed, the workers' items; a team
// that does not exist has no round card.
func TestRoundCardPage(t *testing.T) {
	home := fixture(t)
	var got []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		got = args
		return "Round card — team TT-team\nmembers' work not in the round's PR:\n  member: 2 commits, 3 lines, 10 tokens in\n", nil
	})
	_, ts := testServer(t, home)
	code, body := get(t, ts, "/team/TT-team")
	if code != http.StatusOK || !strings.Contains(body, `href="/team/TT-team/round-check"`) {
		t.Fatalf("the team page (%d) links no round card", code)
	}
	code, body = get(t, ts, "/team/TT-team/round-check")
	if code != http.StatusOK || !strings.Contains(body, "member: 2 commits, 3 lines") {
		t.Fatalf("the round card page (%d) does not show the card:\n%s", code, body)
	}
	if strings.Join(got, " ") != "round-check TT-team" {
		t.Fatalf("agent called with %q, want round-check TT-team", got)
	}
	if code, _ = get(t, ts, "/team/nope/round-check"); code != http.StatusNotFound {
		t.Fatalf("an unknown team's round card: %d, want 404", code)
	}
}
