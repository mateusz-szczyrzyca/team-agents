package main

// Tests of remote jobs on the lead's web interface (written first): the dashboard shows a running job as
// "agent@node" with what it is doing, like a local task; the job's page shows its latest activity and a link to the
// task on its node.

import (
	"fmt"
	"path/filepath"
	"testing"
	"time"
)

// remoteJobs writes a lead's jobs.json with one running job on node-a and its activity.
func remoteJobs(t *testing.T, home string) {
	t.Helper()
	now := time.Now().UTC().Format(time.RFC3339Nano)
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), fmt.Sprintf(`{"Version":1,"Jobs":[
 {"Request":{"ID":"remote-1","Repository":"demo","Selector":"worker-1"},"State":"running","Node":"node-a","TaskID":"T remote","Branch":"agents/worker-1/x","History":[{"State":"running","At":%q}]}],
 "Nodes":{"node-a":{"Heartbeat":{"Node":"node-a","WebURL":"https://example.com/node/","Activity":[{"ID":"remote-1","Agent":"worker-1","Stage":"running","Lines":["10:00:01 make gate","10:00:05 checking the parser"]}]},"LastSeen":%q}}}`, now, now))
}

func TestDashboardShowsRemoteJobsAsAgentAtNode(t *testing.T) {
	home := fixture(t)
	remoteJobs(t, home)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/")
	mustContain(t, "/", body, "worker-1@node-a", "checking the parser", `href="/jobs/remote-1"`)
}

func TestJobPageShowsItsActivity(t *testing.T) {
	home := fixture(t)
	remoteJobs(t, home)
	_, ts := testServer(t, home)
	_, body := get(t, ts, "/jobs/remote-1")
	mustContain(t, "/jobs/remote-1", body, "remote-1", "running", "node-a", "worker-1",
		"10:00:01 make gate", "10:00:05 checking the parser", `href="https://example.com/node/task/T%20remote"`)
}
