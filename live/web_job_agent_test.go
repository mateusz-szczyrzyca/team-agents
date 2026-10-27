package main

import (
	"fmt"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// Tests of web view job attribution to actual chosen agents after terminal completion (docs/tdd.md):
// once a tier-selected job finishes, live activity is dropped; webJob.Agent retains the actual agent,
// links to its detail page, shows it in recent jobs, and avoids bogus /agents/worker@node links on legacy jobs.

func TestWebJobPersistedAgentAttribution(t *testing.T) {
	stubLocalAgents(t)
	home := fixture(t)
	now := time.Now().UTC().Format(time.RFC3339Nano)

	// Node has configured agent worker-1.
	// Job 1: terminal completed tier-selected job (Selector: worker) with persisted Agent: worker-1.
	// Job 2: legacy terminal job (Selector: worker) with no Agent field.
	// Note: Nodes has NO Activity for either job (activity is dropped once terminal).
	data := fmt.Sprintf(`{"Version":1,
 "Jobs":[
  {"Request":{"ID":"job-terminal-worker","Repository":"demo","Selector":"worker"},"State":"done","Node":"node-online","Agent":"worker-1","TaskID":"task-w1","Result":"all tests passed","History":[{"State":"done","At":%q}]},
  {"Request":{"ID":"job-legacy-worker","Repository":"demo","Selector":"worker"},"State":"done","Node":"node-online","Agent":"","TaskID":"task-leg","Result":"legacy done","History":[{"State":"done","At":%q}]}
 ],
 "Nodes":{
  "node-online":{
   "Heartbeat":{
    "Node":"node-online",
    "Agents":[{"Name":"worker-1","Type":"opencode","Tier":"worker"}],
    "FreeAgents":[{"Name":"worker-1","Tier":"worker"}]
   },
   "LastSeen":%q
  }
 }}`, now, now, now)

	writeFile(t, filepath.Join(home, "bus", "jobs.json"), data)
	_, ts := testServer(t, home)

	// 1. Agent detail view: /agents/worker-1@node-online lists job-terminal-worker in its recent work table.
	_, bodyAgent := get(t, ts, "/agents/worker-1@node-online")
	mustContain(t, "/agents/worker-1@node-online", bodyAgent, `href="/jobs/job-terminal-worker"`, "job-terminal-worker")

	// 2. Job view: /jobs/job-terminal-worker links to actual agent /agents/worker-1@node-online.
	_, bodyJob := get(t, ts, "/jobs/job-terminal-worker")
	mustContain(t, "/jobs/job-terminal-worker", bodyJob, `href="/agents/worker-1@node-online"`, "worker-1")

	// 3. Instance view: /instances/node-online links to /agents/worker-1@node-online for job-terminal-worker.
	_, bodyInstance := get(t, ts, "/instances/node-online")
	mustContain(t, "/instances/node-online", bodyInstance, `href="/agents/worker-1@node-online"`, "worker-1")

	// 4. Legacy job view: /jobs/job-legacy-worker has unknown agent for tier selector "worker";
	// must NOT create a bogus link to /agents/worker@node-online (which would 404).
	_, bodyLegacy := get(t, ts, "/jobs/job-legacy-worker")
	if strings.Contains(bodyLegacy, `href="/agents/worker@node-online"`) {
		t.Fatalf("/jobs/job-legacy-worker contains bogus link to /agents/worker@node-online:\n%s", bodyLegacy)
	}
	mustContain(t, "/jobs/job-legacy-worker", bodyLegacy, "worker")
}
