package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestLeadNotificationTitleStartsWithItsStateIcon: a device shows a notification's title first — the lead's starts
// with the icon of the job's state; its Status line is as before.
func TestLeadNotificationTitleStartsWithItsStateIcon(t *testing.T) {
	b := newBus(t)
	notifyDir := t.TempDir()
	a := &leadAPI{bus: b, ttl: time.Minute, notifyDir: notifyDir}
	req := regRequest("job-icon")
	req.Requester = "human"
	req.Repository = "test-repo"
	job, err := a.submit("node-a", true, req)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.cancel(true, job.Request.ID); err != nil {
		t.Fatal(err)
	}
	data, rerr := os.ReadFile(filepath.Join(notifyDir, "job-icon-summary.md")) //nolint:gosec // G304: file path inside t.TempDir()
	if rerr != nil {
		t.Fatal(rerr)
	}
	if first := strings.SplitN(string(data), "\n", 2)[0]; first != "⏹ test-repo" {
		t.Fatalf("first line = %q, want %q", first, "⏹ test-repo")
	}
	if !strings.Contains(string(data), "Status: stopped") {
		t.Fatalf("no Status: stopped in %q", data)
	}
}
