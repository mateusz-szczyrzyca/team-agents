package main

import (
	"fmt"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestProxyRetainsMirrorOfflineWarning checks both displays across heartbeat expiry and recovery.
func TestProxyRetainsMirrorOfflineWarning(t *testing.T) {
	home := fixture(t)
	mirrorFixture(t, home)
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"), "ID=T_PROXY\nAGENT=worker-1@node-a\nKIND=remote-member\nSTATUS=running\nJOB_ID=remote-1\nTEAM=TT-team\n")
	writeFile(t, filepath.Join(home, "tasks", "T_OTHER", "meta.env"), "ID=T_OTHER\nAGENT=worker-1@node-a\nKIND=remote-member\nSTATUS=running\nJOB_ID=remote-1\nTEAM=OTHER-team\n")
	for _, offline := range []bool{true, false} {
		seen := time.Now().UTC()
		if offline {
			seen = seen.Add(-time.Hour)
		}
		writeFile(t, filepath.Join(home, "bus", "jobs.json"), fmt.Sprintf(`{"Version":1,"Jobs":[{"Request":{"ID":"remote-1","Team":"TT-team"},"State":"running","Node":"node-a","TaskID":"T1"}],"Nodes":{"node-a":{"LastSeen":%q}}}`, seen.Format(time.RFC3339Nano)))
		snap := loadSnapshot(home, filepath.Join(home, "bus"))
		proxy := snap.All["T_PROXY"]
		if proxy.NodeOffline != offline || snap.All["T_OTHER"].NodeOffline {
			t.Errorf("offline=%v: matching proxy=%v, other team=%v", offline, proxy.NodeOffline, snap.All["T_OTHER"].NodeOffline)
		}
		if proxy.Status != "running" || !proxy.Active() {
			t.Fatal("heartbeat changed proxy lifecycle")
		}
		row := makeRow(snap, proxy)
		panel := (model{snap: snap, width: 120, height: 40}).row(proxy, 120)
		if strings.Contains(row.StatusLabel, "node offline") != offline || strings.Contains(panel, "node offline") != offline {
			t.Errorf("offline=%v: web=%q panel=%q", offline, row.StatusLabel, panel)
		}
	}
}

// TestJobPagesLinkAvailableMirror checks independent mirror and proxy navigation without guessing a task.
func TestJobPagesLinkAvailableMirror(t *testing.T) {
	for _, proxy := range []bool{false, true} {
		t.Run(fmt.Sprintf("proxy=%v", proxy), func(t *testing.T) {
			home := fixture(t)
			mirrorFixture(t, home)
			writeFile(t, filepath.Join(home, "bus", "jobs.json"), `{"Version":1,"Jobs":[{"Request":{"ID":"remote-1","Team":"TT-team"},"State":"running","Node":"node-a","TaskID":"T1"},{"Request":{"ID":"missing"},"State":"running","Node":"node-a","TaskID":"absent"}],"Nodes":{}}`)
			if proxy {
				writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"), "ID=T_PROXY\nKIND=remote-member\nSTATUS=running\nJOB_ID=remote-1\nTEAM=TT-team\n")
			}
			_, ts := testServer(t, home)
			for _, path := range []string{"/jobs", "/jobs/remote-1"} {
				status, body := get(t, ts, path)
				if status != 200 || !strings.Contains(body, `href="/task/T1@node-a"`) {
					t.Errorf("%s: status=%d, mirror link missing", path, status)
				}
				if strings.Contains(body, `href="/task/T_PROXY"`) != proxy {
					t.Errorf("%s: proxy link presence differs from proxy=%v", path, proxy)
				}
				if strings.Contains(body, `href="/task/absent@node-a"`) {
					t.Errorf("%s links an unavailable mirror", path)
				}
			}
		})
	}
}
