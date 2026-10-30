package main

import (
	"fmt"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// Notifications that could not be delivered in the last day (notify-failures.jsonl, kept by agent): every page says
// how many, the last one's time, process and error; older ones and none at all give no warning.
func TestPagesWarnOfNotificationsNotDeliveredInTheLastDay(t *testing.T) {
	home := fixture(t)
	now := time.Now().Unix()
	writeFile(t, filepath.Join(home, "notify-failures.jsonl"), fmt.Sprintf(
		`{"at":"2026-10-29 23:00","epoch":%d,"from":"_fixpass","command":"./ntfy.sh","error":"exit 1: old"}`+"\n"+
			`{"at":"2026-10-30 10:44","epoch":%d,"from":"_remote_member","command":"./ntfy.sh","error":"exit 1: first"}`+"\n"+
			`{"at":"2026-10-30 10:54","epoch":%d,"from":"_merge","command":"./ntfy.sh","error":"exit 1: ./ntfy.sh: line 11: NTFY_URL: set NTFY_URL"}`+"\n",
		now-90000, now-600, now-60))
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, "2 notifications not delivered in the last 24 h", "last at 2026-10-30 10:54 from _merge", "NTFY_URL: set NTFY_URL")

	clean := fixture(t)
	_, ts2 := testServer(t, clean)
	if _, body := get(t, ts2, "/"); strings.Contains(body, "not delivered") {
		t.Errorf("a warning without notify-failures.jsonl")
	}
}
