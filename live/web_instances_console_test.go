package main

import (
	"encoding/json"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestInstancesConsoleLinksAndVisibility keeps old offline nodes accessible without crowding the default list.
func TestInstancesConsoleLinksAndVisibility(t *testing.T) {
	home := fixture(t)
	now := time.Now().UTC()
	snapshot := webJobsSnapshot{Version: 1, Nodes: map[string]webJobNode{
		"current":        {LastSeen: now, Heartbeat: webJobHeartbeat{FreeAgents: []webJobAgent{{Name: "worker-one", Tier: "worker"}}}},
		"recent-offline": {LastSeen: now.Add(-23 * time.Hour)},
		"old-offline":    {LastSeen: now.Add(-25 * time.Hour)},
		"never-seen":     {},
	}}
	data, err := json.Marshal(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), string(data))
	server, ts := testServer(t, home)
	code, body := get(t, ts, "/instances")
	if code != http.StatusOK {
		t.Fatalf("status %d", code)
	}
	mustContain(t, "default instances", body, `href="/agents/worker-one@current"`, `href="/instances/recent-offline"`, `href="/instances/never-seen"`, `href="/instances?all=1"`)
	if strings.Contains(body, `href="/instances/old-offline"`) {
		t.Error("old offline node visible by default")
	}
	code, body = get(t, ts, "/instances?all=1")
	if code != http.StatusOK {
		t.Fatalf("all status %d", code)
	}
	mustContain(t, "all instances", body, `href="/instances/old-offline"`, `hx-get="/instances?all=1"`)
	code, _ = get(t, ts, "/instances/old-offline")
	if code != http.StatusOK {
		t.Fatalf("hidden instance detail status %d", code)
	}
	server.cfg.nodeTTL = 48 * time.Hour
	_, body = get(t, ts, "/instances")
	mustContain(t, "old online node", body, `href="/instances/old-offline"`)
}
