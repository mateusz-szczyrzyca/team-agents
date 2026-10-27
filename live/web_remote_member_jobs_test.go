package main

import (
	"fmt"
	"net/http"
	"path/filepath"
	"testing"
	"time"
)

// TestJobsRemoteMemberJobLinksToTeam verifies that remote team member jobs link to their team on /jobs and /jobs/{id}.
func TestJobsRemoteMemberJobLinksToTeam(t *testing.T) {
	home := fixture(t)
	now := time.Now().UTC()

	// 1. A remote member job with a proxy task on the lead
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"),
		"ID=T_PROXY\nAGENT=worker-1@node-online\nSTATUS=running\nRUN=1\nKIND=remote-member\nTEAM=TT-team\nJOB_ID=job-proxy-member\n")

	// 2. A remote member job with Team set in Request directly
	jobsJSON := fmt.Sprintf(`{"Version":1,"Jobs":[
		{"Request":{"ID":"job-proxy-member","Repository":"team-agents","Selector":"worker-1"},"State":"running","Node":"node-online","TaskID":"task-101"},
		{"Request":{"ID":"job-direct-member","Repository":"team-agents","Selector":"worker-2","Team":"TT-team"},"State":"running","Node":"node-online","TaskID":"task-102"}
	],"Nodes":{"node-online":{"Heartbeat":{"Node":"node-online","WebURL":"https://example.com/node/"},"LastSeen":%q}}}`,
		now.Format(time.RFC3339Nano))
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), jobsJSON)

	_, ts := testServer(t, home)

	// GET /jobs
	code, body := get(t, ts, "/jobs")
	if code != http.StatusOK {
		t.Fatalf("/jobs: status %d, body %.200s", code, body)
	}
	mustContain(t, "/jobs", body,
		`href="/jobs/job-proxy-member"`,
		`href="/team/TT-team"`,
		`href="/task/T_PROXY"`,
		`href="/jobs/job-direct-member"`,
	)

	// GET /jobs/job-proxy-member
	codeProxy, bodyProxy := get(t, ts, "/jobs/job-proxy-member")
	if codeProxy != http.StatusOK {
		t.Fatalf("/jobs/job-proxy-member: status %d, body %.200s", codeProxy, bodyProxy)
	}
	mustContain(t, "/jobs/job-proxy-member", bodyProxy,
		`href="/team/TT-team"`,
		`href="/task/T_PROXY"`,
	)

	// GET /jobs/job-direct-member
	codeDirect, bodyDirect := get(t, ts, "/jobs/job-direct-member")
	if codeDirect != http.StatusOK {
		t.Fatalf("/jobs/job-direct-member: status %d, body %.200s", codeDirect, bodyDirect)
	}
	mustContain(t, "/jobs/job-direct-member", bodyDirect,
		`href="/team/TT-team"`,
	)
}
