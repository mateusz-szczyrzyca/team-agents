package main

import (
	"fmt"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestMirrorHistoryAgentLink checks the remote agent row selects the mirrored task URL.
func TestMirrorHistoryAgentLink(t *testing.T) {
	home := fixture(t)
	stubLocalAgents(t)
	dir := filepath.Join(home, "bus", "remote", "node-a", "remote")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{"AGENT":"worker-1","STATUS":"running","JOB_ID":"remote-1"}`)
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), fmt.Sprintf(`{"Version":1,"Jobs":[{"Request":{"ID":"remote-1","Selector":"worker-1"},"State":"running","Node":"node-a","TaskID":"remote"}],"Nodes":{"node-a":{"LastSeen":%q,"Heartbeat":{"Node":"node-a","Agents":[{"Name":"worker-1","Type":"codex","Tier":"worker"}]}}}}`, time.Now().UTC().Format(time.RFC3339Nano)))
	_, ts := testServer(t, home)
	code, body := get(t, ts, "/agents")
	if code != 200 || !strings.Contains(body, `href="/task/remote@node-a"`) {
		t.Fatal("remote agent row does not link to mirrored task")
	}
}
