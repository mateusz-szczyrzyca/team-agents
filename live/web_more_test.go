package main

// Tests of the web interface's second round (written first, docs/tdd.md): investigators named on the dashboard,
// agents by activity, the teams list cut at 20, the board newest first, waking a finished team, the bus newest first
// in colours by direction, the investigator's model, the stop button, and the status colours.

import (
	"fmt"
	"net/http"
	"net/url"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"
)

// addTask writes one more task into a fixture home.
func addTask(t *testing.T, home, id, meta string) {
	t.Helper()
	writeFile(t, filepath.Join(home, "tasks", id, "meta.env"), "ID="+id+"\nTYPE=claude\nREPO_NAME=repo\nBRANCH=agents/x\nCREATED=2026-10-23T08:00:00+02:00\n"+meta)
}

// before reports whether a appears in body before b (both must appear).
func before(body, a, b string) bool {
	i, j := strings.Index(body, a), strings.Index(body, b)
	return i >= 0 && j >= 0 && i < j
}

func TestDashboardNamesInvestigators(t *testing.T) {
	home := fixture(t)
	addTask(t, home, "T6", "AGENT=inv-astra\nKIND=investigation\nINV_ID=7\nSTATUS=running\nRUN=1\n")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, "investigator #7 (inv-astra)")
}

func TestAgentsAreOrderedByActivity(t *testing.T) {
	home := fixture(t)
	now := time.Now()
	addTask(t, home, "T7", "AGENT=recent\nSTATUS=done\nRUN=1\nFINISHED="+now.Add(-10*time.Minute).Format(time.RFC3339)+"\n")
	addTask(t, home, "T8", "AGENT=old\nSTATUS=done\nRUN=1\nFINISHED="+now.Add(-48*time.Hour).Format(time.RFC3339)+"\n")
	stubAgent(t, func(_ ...string) (string, *OpError[AgentCallCause]) {
		return `{"name":"old","type":"claude","tier":"worker"}` + "\n" + `{"name":"recent","type":"claude","tier":"worker"}` + "\n" +
			`{"name":"solo","type":"claude","tier":"normal"}` + "\n" + `{"name":"idle","type":"claude","tier":"worker"}` + "\n", nil
	})
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/agents")
	// working first (solo runs T1), then by last activity, never-active last
	if !before(body, ">solo<", ">recent<") || !before(body, ">recent<", ">old<") || !before(body, ">old<", ">idle<") {
		t.Errorf("/agents order wrong")
	}
	mustContain(t, "/agents", body, "Last activity", now.Add(-10*time.Minute).Format("2006-01-02 15:04"))
}

// teamsState writes a bus state with n finished teams (team-01 … team-NN) plus the fixture's TT-team.
func teamsState(t *testing.T, home string, n int) {
	t.Helper()
	var b strings.Builder
	b.WriteString(`{"teams": {"TT-team": {"id": "TT-team", "lead": "lead", "done": true, "members": {"lead": {"state": "done", "tier": "normal", "task": "T4"}},
	 "board": {"B1": {"id": "B1", "title": "first item", "status": "done"}, "B2": {"id": "B2", "title": "second item", "status": "todo"},
	           "B10": {"id": "B10", "title": "tenth item", "status": "todo"}}}`)
	for i := 1; i <= n; i++ {
		fmt.Fprintf(&b, `, "2026101%d-0000%02d-team": {"id": "2026101%d-0000%02d-team", "lead": "lead", "done": true, "members": {}}`, i%10, i, i%10, i)
	}
	b.WriteString(`}}`)
	writeFile(t, filepath.Join(home, "bus", "state.json"), b.String())
}

func TestTeamsListShowsTwentyAndExpands(t *testing.T) {
	home := fixture(t)
	teamsState(t, home, 25)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/teams")
	if n := strings.Count(body, `href="/team/`); n != 20 {
		t.Errorf("/teams lists %d teams, want 20", n)
	}
	mustContain(t, "/teams", body, `href="/teams?all=1"`, "all 26 teams")
	_, all := get(t, ts, "/teams?all=1")
	if n := strings.Count(all, `href="/team/`); n != 26 {
		t.Errorf("/teams?all=1 lists %d teams, want 26", n)
	}
}

func TestTeamBoardNewestFirstAndAFinishedTeamCanBeWoken(t *testing.T) {
	home := fixture(t)
	teamsState(t, home, 0)
	var called []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		called = args
		return "Resumed TT-team from the last checkpoint", nil
	})
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/team/TT-team")
	if !before(body, "tenth item", "second item") || !before(body, "second item", "first item") {
		t.Errorf("board not newest first")
	}
	mustContain(t, "/team", body, `action="/team/TT-team/resume"`, `value="same"`, `value="next"`)
	code, out := post(t, ts, "/team/TT-team/resume", url.Values{"mode": {"same"}, "prompt": {"fix the review"}})
	if code != http.StatusOK || strings.Join(called, " ") != "run --resume TT-team --same-round fix the review" ||
		!strings.Contains(out, "Resumed TT-team") {
		t.Fatalf("resume same: %d %v", code, called)
	}
	post(t, ts, "/team/TT-team/resume", url.Values{"mode": {"next"}, "prompt": {""}})
	if strings.Join(called, " ") != "run --resume TT-team" {
		t.Errorf("resume next: %v", called)
	}
}

func TestBusNewestFirstInColoursByDirection(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "bus", "messages.jsonl"), strings.Join([]string{
		`{"id":3,"ts":"2026-10-23 08:02:00","from":"codex-1","to":"gemini-1","kind":"chat","text":"ping one"}`,
		`{"id":4,"ts":"2026-10-23 08:03:00","from":"gemini-1","to":"codex-1","kind":"chat","text":"pong two"}`,
		`{"id":5,"ts":"2026-10-23 08:04:00","from":"codex-1","to":"gemini-1","kind":"chat","text":"ping three"}`,
	}, "\n")+"\n")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/bus")
	if !before(body, "ping three", "pong two") || !before(body, "pong two", "ping one") {
		t.Errorf("/bus not newest first")
	}
	class := func(text string) string {
		m := regexp.MustCompile(`<li class="[^"]*\b(pair-[0-9]+)\b[^"]*">(?:[^<]|<[^/]|</[^l])*?` + regexp.QuoteMeta(text)).FindStringSubmatch(body)
		if m == nil {
			t.Fatalf("no pair class for %q", text)
		}
		return m[1]
	}
	if class("ping one") != class("ping three") || class("ping one") == class("pong two") {
		t.Errorf("pair colours: %s %s %s", class("ping one"), class("ping three"), class("pong two"))
	}
	mustContain(t, "/bus", body, `hx-swap="afterbegin"`)
}

func TestInvestigationsNameTheInvestigatorAndModel(t *testing.T) {
	home := fixture(t)
	addTask(t, home, "T9", "AGENT=inv-astra\nKIND=investigation\nINV_ID=7\nMODEL=gpt-6-astra\nSTATUS=done\nRUN=1\n")
	writeFile(t, filepath.Join(home, "bus", "state.json"), `{"investigations": {"7": {"id": 7, "from_agent": "solo", "from_task": "T1",
	  "question": "Is DMA safe here?", "status": "done", "summary": "yes, with a fence", "inv_task": "T9"}}}`)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/investigations")
	mustContain(t, "/investigations", body, "inv-astra", "gpt-6-astra")
}

func TestStopButtonKicksARunningAgent(t *testing.T) {
	home := fixture(t)
	var called []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		called = args
		return "stopped", nil
	})
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, `hx-post="/task/T1/stop"`, `hx-confirm=`)
	if strings.Contains(body, `hx-post="/task/T3/stop"`) {
		t.Error("a finished task has a stop button")
	}
	if code, _ := post(t, ts, "/task/T1/stop", url.Values{}); code != http.StatusOK || strings.Join(called, " ") != "kick T1 web" {
		t.Errorf("stop: %d %v", code, called)
	}
}

func TestStatusColours(t *testing.T) {
	for st, want := range map[string]string{"stopped": "st-stopped", "failed": "st-failed", "hung": "st-failed", "dead": "st-failed", "excluded": "st-failed"} {
		if got := statusClass(st); got != want {
			t.Errorf("statusClass(%s) = %s, want %s", st, got, want)
		}
	}
}
