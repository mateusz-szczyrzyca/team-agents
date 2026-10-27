package main

// Tests for Round 1: remote instances, agents, job links and dashboard finished jobs (Decisions 5–8).
// Written first: fails until handlers, templates and styles are fully implemented.

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// setupRound1Bus writes a jobs.json fixture with online, offline, and never-seen nodes,
// configured agents, and jobs with GitHub RepoURL and RequesterTask.
func setupRound1Bus(t *testing.T, home string) {
	t.Helper()
	now := time.Now().UTC()
	nowStr := now.Format(time.RFC3339Nano)
	oldSeen := now.Add(-2 * time.Hour).Format(time.RFC3339Nano)
	finishedRecent := now.Add(-15 * time.Minute).Format(time.RFC3339Nano)
	finishedOld := now.Add(-30 * time.Hour).Format(time.RFC3339Nano)

	snap := webJobsSnapshot{
		Version: 1,
		Jobs: []webJob{
			{
				Request: webJobRequest{
					ID:            "job-active",
					Repository:    "team-agents",
					Selector:      "worker-1",
					Requires:      []string{"fast"},
					RepoURL:       "https://github.com/example/team-agents",
					RequesterTask: "T1",
				},
				State:  "running",
				Node:   "node-online",
				TaskID: "task-101",
				Branch: "agents/worker-1/round1",
				History: []webJobTransition{
					{State: "running", At: now.Add(-5 * time.Minute)},
				},
			},
			{
				Request: webJobRequest{
					ID:            "job-recent-done",
					Repository:    "team-agents",
					Selector:      "worker-1",
					RepoURL:       "https://github.com/example/team-agents",
					RequesterTask: "T4",
				},
				State:  "done",
				Node:   "node-online",
				TaskID: "task-102",
				Branch: "agents/worker-1/done-branch",
				Result: "All checks passed",
				History: []webJobTransition{
					{State: "running", At: now.Add(-25 * time.Minute)},
					{State: "done", At: now.Add(-15 * time.Minute)},
				},
			},
			{
				Request: webJobRequest{
					ID:         "job-non-gh",
					Repository: "internal-repo",
					Selector:   "worker-b",
					RepoURL:    "https://gitlab.example.com/team/internal-repo",
				},
				State:  "running",
				Node:   "node-offline",
				TaskID: "task-201",
				Branch: "main",
			},
			{
				Request: webJobRequest{
					ID:         "job-old-done",
					Repository: "team-agents",
					Selector:   "worker-1",
				},
				State:  "done",
				Node:   "node-online",
				TaskID: "task-103",
				Result: "Finished yesterday",
				History: []webJobTransition{
					{State: "done", At: now.Add(-30 * time.Hour)},
				},
			},
		},
		Nodes: map[string]webJobNode{
			"node-online": {
				Heartbeat: webJobHeartbeat{
					Node:         "node-online",
					Capabilities: []string{"fast", "hw:gpu"},
					Agents: []webNodeAgent{
						{Name: "worker-1", Type: "claude", Model: "claude-3-5-haiku", Tier: "worker"},
						{Name: "agent-free", Type: "codex", Model: "gpt-5", Tier: "normal"},
					},
					FreeAgents: []webJobAgent{
						{Name: "agent-free", Tier: "normal"},
					},
					WebURL: "https://example.com/node-online/",
					Activity: []webJobActivity{
						{
							ID:    "job-active",
							Agent: "worker-1",
							Stage: "running",
							Lines: []string{"16:50:00 gate passed", "16:55:00 testing views"},
						},
					},
				},
				LastSeen: now,
			},
			"node-offline": {
				Heartbeat: webJobHeartbeat{
					Node:         "node-offline",
					Capabilities: []string{"arm64"},
					Agents: []webNodeAgent{
						{Name: "worker-b", Type: "gemini", Model: "gemini-flash", Tier: "worker"},
					},
					WebURL: "",
				},
				LastSeen: now.Add(-2 * time.Hour),
			},
			"node-unseen": {
				Heartbeat: webJobHeartbeat{
					Node: "node-unseen",
				},
			},
		},
	}

	data, err := json.Marshal(snap)
	if err != nil {
		t.Fatal(err)
	}
	// Format with custom RFC3339 timestamps for compatibility
	_ = nowStr
	_ = oldSeen
	_ = finishedRecent
	_ = finishedOld
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), string(data))
}

func TestInstancesStatusColoursAndLink(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	_, ts := testServer(t, home)

	_, body := get(t, ts, "/instances")

	// Instance name links to /instances/<node>
	mustContain(t, "/instances", body, `href="/instances/node-online"`, ">node-online<")
	mustContain(t, "/instances", body, `href="/instances/node-offline"`, ">node-offline<")
	mustContain(t, "/instances", body, `href="/instances/node-unseen"`, ">node-unseen<")

	// Status colours: online green, offline red, never seen/unknown grey
	mustContain(t, "/instances", body, `class="st-online"`, ">online<")
	mustContain(t, "/instances", body, `class="st-offline"`, ">offline<")
	mustContain(t, "/instances", body, `class="st-dim"`, ">never seen<")
}

func TestInstanceDetailPage(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	_, ts := testServer(t, home)

	// Detail page of node-online
	_, body := get(t, ts, "/instances/node-online")
	mustContain(t, "/instances/node-online", body, "node-online", "fast", "hw:gpu")
	// All agents, linking to agent page
	mustContain(t, "/instances/node-online", body, `href="/agents/worker-1@node-online"`, "worker-1")
	mustContain(t, "/instances/node-online", body, `href="/agents/agent-free@node-online"`, "agent-free")
	// Free agents
	mustContain(t, "/instances/node-online", body, "agent-free")
	// Assigned jobs
	mustContain(t, "/instances/node-online", body, `href="/jobs/job-active"`, "job-active")
	mustContain(t, "/instances/node-online", body, `href="/jobs/job-recent-done"`, "job-recent-done")
	// Link to node's own web interface
	mustContain(t, "/instances/node-online", body, `href="https://example.com/node-online/"`)

	// Non-existent node returns 404
	status, _ := get(t, ts, "/instances/node-does-not-exist")
	if status != http.StatusNotFound {
		t.Fatalf("expected 404 for missing node, got %d", status)
	}
}

// stubLocalAgents stubs callAgent for _agents-json to return a test agent list.
func stubLocalAgents(t *testing.T) {
	t.Helper()
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		if len(args) > 0 && args[0] == "_agents-json" {
			return `{"name":"solo","type":"claude","model":"opus","tier":"normal","provider":"anthropic"}` + "\n" +
				`{"name":"member","type":"codex","model":"gpt-4","tier":"normal","provider":"openai"}` + "\n", nil
		}
		return "", nil
	})
}

func TestAgentsListShowsRemoteAgents(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	stubLocalAgents(t)
	_, ts := testServer(t, home)

	_, body := get(t, ts, "/agents")

	// Remote agents listed as agent@node
	mustContain(t, "/agents", body, "worker-1@node-online", "claude", "claude-3-5-haiku", "worker")
	mustContain(t, "/agents", body, "agent-free@node-online", "codex", "gpt-5", "normal")
	mustContain(t, "/agents", body, "worker-b@node-offline", "gemini", "gemini-flash")

	// Links to agent pages
	mustContain(t, "/agents", body, `href="/agents/solo"`)
	mustContain(t, "/agents", body, `href="/agents/worker-1@node-online"`)

	// What worker-1@node-online is doing now (job-active)
	mustContain(t, "/agents", body, `href="/jobs/job-active"`)
}

func TestAgentDetailPageLocalAndRemote(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	stubLocalAgents(t)
	_, ts := testServer(t, home)

	// Local agent page
	_, bodyLocal := get(t, ts, "/agents/solo")
	mustContain(t, "/agents/solo", bodyLocal, "solo", "claude", `href="/task/T1"`)

	// Remote agent page
	_, bodyRemote := get(t, ts, "/agents/worker-1@node-online")
	mustContain(t, "/agents/worker-1@node-online", bodyRemote, "worker-1@node-online", "claude", "claude-3-5-haiku", "worker")
	mustContain(t, "/agents/worker-1@node-online", bodyRemote, `href="/jobs/job-active"`, `href="/jobs/job-recent-done"`)

	// Missing agent returns 404
	status, _ := get(t, ts, "/agents/unknown-agent")
	if status != http.StatusNotFound {
		t.Fatalf("expected 404 for missing agent, got %d", status)
	}
}

func TestJobsViewLinks(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	_, ts := testServer(t, home)

	_, body := get(t, ts, "/jobs")

	// Node links to /instances/<node>
	mustContain(t, "/jobs", body, `href="/instances/node-online"`, ">node-online<")

	// Agent links to /agents/<agent>@<node>
	mustContain(t, "/jobs", body, `href="/agents/worker-1@node-online"`, ">worker-1<")

	// GitHub branch link: https://github.com/<owner>/<repo>/tree/<branch>
	mustContain(t, "/jobs", body, `href="https://github.com/example/team-agents/tree/agents/worker-1/round1"`)

	// Non-GitHub branch has no tree link
	if strings.Contains(body, `gitlab.example.com/team/internal-repo/tree`) {
		t.Errorf("non-GitHub branch unexpectedly linked to tree")
	}

	// Requester links to task and team on lead
	mustContain(t, "/jobs", body, `href="/task/T1"`, ">T1<")
	mustContain(t, "/jobs", body, `href="/task/T4"`, `href="/team/TT-team"`)
}

func TestJobDetailViewLinks(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	_, ts := testServer(t, home)

	_, body := get(t, ts, "/jobs/job-active")
	mustContain(t, "/jobs/job-active", body, `href="/instances/node-online"`, `href="/agents/worker-1@node-online"`)
	mustContain(t, "/jobs/job-active", body, `href="https://github.com/example/team-agents/tree/agents/worker-1/round1"`)
	mustContain(t, "/jobs/job-active", body, `href="/task/T1"`)
}

func TestDashboardFinishedRemoteJobsAndDoneBlue(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	_, ts := testServer(t, home)

	_, body := get(t, ts, "/")

	// Active remote job appears
	mustContain(t, "/", body, "worker-1@node-online", `href="/jobs/job-active"`)

	// Recent done job appears and links to /jobs/job-recent-done
	mustContain(t, "/", body, `href="/jobs/job-recent-done"`)

	// Old done job (>24h) does NOT appear on dashboard
	if strings.Contains(body, `href="/jobs/job-old-done"`) {
		t.Errorf("remote job finished >24h ago should not appear on dashboard")
	}

	// CSS defines done as blue
	css, err := os.ReadFile(filepath.Join("web", "static", "app.css"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(css)
	// Light mode done blue
	if !strings.Contains(s, "--done: #2563eb;") && !strings.Contains(s, "--done: var(--accent);") {
		t.Errorf("app.css should define --done in blue")
	}
}

func TestDashboardProxyTaskDeduplication(t *testing.T) {
	home := fixture(t)
	setupRound1Bus(t, home)
	// Add a proxy task on the lead representing job-active
	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"),
		"ID=T_PROXY\nAGENT=worker-1@node-online\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=job-active\n")
	_, ts := testServer(t, home)

	_, body := get(t, ts, "/")

	// job-active should appear as the local proxy task (/task/T_PROXY),
	// not duplicated as a remote job row (/jobs/job-active)
	mustContain(t, "/", body, `href="/task/T_PROXY"`)
	if strings.Contains(body, `href="/jobs/job-active"`) {
		t.Errorf("job-active should be represented by its proxy task, not duplicated as a remote job row")
	}
}
