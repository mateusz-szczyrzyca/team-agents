package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestIndependentInvestigatorsShowTheirNames verifies that the investigators of an --investigation team appear on the
// dashboard and in the panel under their own names: they have no investigation number and work for no agent, so the
// legacy "investigator #N (name)" / "inv#N" labels and the "for <agent>" stage do not apply to them.
func TestIndependentInvestigatorsShowTheirNames(t *testing.T) {
	home := fixture(t)
	now := time.Now().Format(time.RFC3339)
	parent := "20261102-201616-investigation-team"
	addTask(t, home, parent, "AGENT=investigation-team\nKIND=investigation_team\nSTATUS=running\nRUN=0\nCREATED="+now+"\n")
	// Phase 1: no INV_ID at all; phase 2: INV_ID is the child's own task ID.
	addTask(t, home, "20261102-201616-inv-opus", "AGENT=inv-opus\nKIND=investigation\nINV_ID=\nINVESTIGATION_TEAM="+parent+
		"\nINVESTIGATION_PHASE=1\nPARENT_TASK="+parent+"\nSTATUS=running\nRUN=1\nCREATED="+now+"\n")
	addTask(t, home, "20261102-211616-inv-astra", "AGENT=inv-astra\nKIND=investigation\nINV_ID=20261102-211616-inv-astra\nINVESTIGATION_TEAM="+parent+
		"\nINVESTIGATION_PHASE=2\nPARENT_TASK="+parent+"\nSTATUS=running\nRUN=1\nCREATED="+now+"\n")
	// A legacy investigation requested by an agent keeps its number and requester.
	addTask(t, home, "TLEGACY", "AGENT=claude-inv\nKIND=investigation\nINV_ID=10\nFROM_AGENT=solo\nSTATUS=running\nRUN=1\nCREATED="+now+"\n")

	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, `/task/20261102-201616-inv-opus">inv-opus</a>`, `/task/20261102-211616-inv-astra">inv-astra</a>`,
		"investigator #10 (claude-inv)", `<span class="stage">for solo</span>`)
	mustNotContain(t, "/", body, "investigator # (", "investigator #20261102-211616-inv-astra", `<span class="stage">for</span>`,
		"investigation-team")

	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	for id, want := range map[string]string{"20261102-201616-inv-opus": "inv-opus", "20261102-211616-inv-astra": "inv-astra", "TLEGACY": "inv#10"} {
		task := snap.All[id]
		if task == nil {
			t.Fatalf("snap.All misses %s", id)
		}
		if got := task.Label(); got != want {
			t.Errorf("Label(%s) = %q, want %q", id, got, want)
		}
	}
	panel := (model{snap: snap}).row(snap.All["20261102-201616-inv-opus"], 160)
	if !strings.Contains(panel, "inv-opus") || strings.Contains(panel, "inv#") || strings.Contains(panel, "for ") {
		t.Errorf("panel row of an independent investigator: %q", panel)
	}
}

// TestWatcherNamesNewIndependentInvestigators verifies that the panel's event line for a new independent investigator
// names it and does not say it works "for" an agent.
func TestWatcherNamesNewIndependentInvestigators(t *testing.T) {
	home := fixture(t)
	w := NewWatcher(home, filepath.Join(home, "bus"), 0)
	w.Poll() // the first poll only records the baseline
	addTask(t, home, "20261102-201616-inv-opus", "AGENT=inv-opus\nKIND=investigation\nINV_ID=\nINVESTIGATION_TEAM=20261102-201616-investigation-team\nSTATUS=starting\nRUN=1\n")
	writeFile(t, filepath.Join(home, "tasks", "20261102-201616-inv-opus", "task.md"), "Fix the investigation option\n")
	_, events := w.Poll()
	found := false
	for _, ev := range events {
		if strings.Contains(ev.Text, "new task inv-opus [repo]: Fix the investigation option") {
			found = true
		}
		if strings.Contains(ev.Text, "inv#") || strings.Contains(ev.Text, "for : ") {
			t.Errorf("event names an independent investigator as a legacy one: %q", ev.Text)
		}
	}
	if !found {
		t.Errorf("no event for the new independent investigator: %+v", events)
	}
}
