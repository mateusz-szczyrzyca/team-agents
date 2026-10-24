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
