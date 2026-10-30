package main

import (
	"path/filepath"
	"strings"
	"testing"
)

// While the notification command fails (notify-failed.json, kept by agent until a notification is delivered),
// every page says so, with since when and why; without the file, none does.
func TestPagesWarnWhileNotificationsAreNotDelivered(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "notify-failed.json"),
		`{"since":"2026-10-30 00:44","command":"./ntfy.sh","error":"exit 1: ./ntfy.sh: line 11: NTFY_URL: set NTFY_URL"}`)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, "notifications are not delivered", "since 2026-10-30 00:44", "NTFY_URL: set NTFY_URL")

	clean := fixture(t)
	_, ts2 := testServer(t, clean)
	if _, body := get(t, ts2, "/"); strings.Contains(body, "notifications are not delivered") {
		t.Errorf("a warning without notify-failed.json")
	}
}
