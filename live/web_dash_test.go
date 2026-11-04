package main

// Tests of the dashboard's rows and the bus colours (written first): finished tasks newest first after the active
// ones, recently finished investigations shown even when acknowledged, "… ago" for finished tasks, and pair colour
// rules that outrank the base rule of the message list.

import (
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestDashboardShowsRecentFinishedTasksNewestFirst(t *testing.T) {
	home := fixture(t)
	now := time.Now()
	addTask(t, home, "TOLD", "AGENT=old-one\nSTATUS=stopped\nRUN=1\nFINISHED="+now.Add(-30*time.Hour).Format(time.RFC3339)+"\n")
	addTask(t, home, "TNEW", "AGENT=new-one\nSTATUS=done\nRUN=1\nFINISHED="+now.Add(-5*time.Minute).Format(time.RFC3339)+"\n")
	addTask(t, home, "TINV", "AGENT=inv-astra\nKIND=investigation\nINV_ID=30\nSTATUS=done\nRUN=1\nFINISHED="+now.Add(-10*time.Minute).Format(time.RFC3339)+"\n")
	writeFile(t, filepath.Join(home, "tasks", "TINV", "acked"), "")
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, "investigator #30 (inv-astra)", "5m ago")
	// active first (solo is running), then finished newest first
	if !before(body, ">solo<", ">new-one<") || !before(body, ">new-one<", "investigator #30") || !before(body, "investigator #30", ">old-one<") {
		t.Errorf("dashboard order wrong")
	}
}

func TestPairColoursOutrankTheListRule(t *testing.T) {
	css, err := os.ReadFile(filepath.Join("web", "static", "app.css"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(css)
	if regexp.MustCompile(`(?m)^ol\.msgs li \{[^}]*border-left:[^}]*\}`).MatchString(s) {
		t.Error("the list rule sets the border-left shorthand (its colour outranks .pair-N)")
	}
	for i := 0; i < pairColours; i++ {
		if !strings.Contains(s, "ol.msgs li.pair-"+itoa(i)+" ") {
			t.Errorf("pair-%d is not qualified as ol.msgs li.pair-%d", i, i)
		}
	}
}

// itoa writes a small number.
func itoa(i int) string { return strconv.Itoa(i) }

func TestStoppedTeamMemberIsNotShownAsSleeping(t *testing.T) {
	home := fixture(t)
	now := time.Now().Format(time.RFC3339)
	addTask(t, home, "20261104-122828-gemini-1",
		"AGENT=gemini-1\nTYPE=gemini\nMODEL=gemini-3.1-pro\nSTATUS=done\nRUN=1\n"+
			"TEAM=20261104-122828-team\nTEAM_ROLE=coordinator\nCREATED="+now+"\nSTARTED="+now+"\nFINISHED="+now+"\n")
	addTask(t, home, "20261104-122828-claude-haiku",
		"AGENT=claude-haiku\nTYPE=claude\nMODEL=haiku\nSTATUS=stopped\nRUN=1\n"+
			"TEAM=20261104-122828-team\nTEAM_ROLE=member\nCREATED="+now+"\nSTARTED="+now+"\nFINISHED="+now+"\n")
	state := `{"teams":{"20261104-122828-team":{"id":"20261104-122828-team","created":"2026-11-04 12:28:28","done":true,` +
		`"members":{` +
		`"gemini-1":{"role":"coordinator","tier":"normal","type":"gemini","model":"gemini-3.1-pro","task":"20261104-122828-gemini-1","state":"done"},` +
		`"claude-haiku":{"role":"member","tier":"worker","type":"claude","model":"haiku","task":"20261104-122828-claude-haiku","state":"sleeping"}` +
		`},"board":[]}}}`
	writeFile(t, filepath.Join(home, "bus", "state.json"), state)

	_, ts := testServer(t, home)
	for _, path := range []string{"/teams", "/team/20261104-122828-team"} {
		_, body := get(t, ts, path)
		if strings.Contains(body, ">sleeping<") || strings.Contains(body, "1 sleeping") {
			t.Fatalf("%s showed stopped member claude-haiku as sleeping:\n%s", path, body)
		}
	}
	_, teamBody := get(t, ts, "/team/20261104-122828-team")
	mustContain(t, "/team/20261104-122828-team", teamBody, ">stopped<")
	_, teamsBody := get(t, ts, "/teams")
	mustContain(t, "/teams", teamsBody, "1 done, 1 stopped")
}

func TestDashboardShowsAtMost20LatestEntriesAndFullDashboardViewShowsAll(t *testing.T) {
	home := fixture(t)
	now := time.Now()
	for i := 1; i <= 25; i++ {
		id := "20261104-1200" + strconv.Itoa(10+i) + "-task-" + strconv.Itoa(i)
		agent := "agent-" + strconv.Itoa(i)
		fin := now.Add(-time.Duration(i) * time.Minute).Format(time.RFC3339)
		addTask(t, home, id, "AGENT="+agent+"\nSTATUS=stopped\nRUN=1\nFINISHED="+fin+"\n")
	}
	_, ts := testServer(t, home)
	_, dash := get(t, ts, "/")
	mustContain(t, "/", dash, "Full dashboard view", `/dashboard/full`)
	// fixture(t) already has 3 active tasks (solo, helper, wait1), so 3 active + 17 newest stopped = 20 rows on /.
	// agent-1 .. agent-17 must be shown; agent-18 .. agent-25 must NOT be shown on /.
	for i := 1; i <= 17; i++ {
		mustContain(t, "/", dash, ">agent-"+strconv.Itoa(i)+"<")
	}
	for i := 18; i <= 25; i++ {
		if strings.Contains(dash, ">agent-"+strconv.Itoa(i)+"<") {
			t.Fatalf("/ showed >20 entries (found agent-%d on default dashboard)", i)
		}
	}

	_, full := get(t, ts, "/dashboard/full")
	mustContain(t, "/dashboard/full", full, "Full dashboard view")
	for i := 1; i <= 25; i++ {
		mustContain(t, "/dashboard/full", full, ">agent-"+strconv.Itoa(i)+"<")
	}
}
