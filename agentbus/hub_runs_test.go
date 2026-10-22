package main

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// runResolverFixture registers a remote normal run and its independent qualified team assignment.
func runResolverFixture(t *testing.T) (hubRunRegistry, HubCaller) {
	t.Helper()
	b := newBus(t)
	registry := &HubInstanceRegistry{Bus: b}
	c := HubCaller{Agent: "codex-1", Instance: "mac", Task: "task-one", Team: "team-one", Tier: TierNormal, Role: RoleAgent}
	req := instanceRegisterFixture("mac", "session-one")
	req.Manifest.CurrentState.Tasks = map[string]HubTaskSummary{c.Task: {TaskID: c.Task, Status: "running", Agent: c.Agent, Team: c.Team, Tier: c.Tier, Role: c.Role, Repo: "example-app", Branch: "agents/codex-1", Worktree: "/remote/worktree"}}
	if _, err := registry.Register("mac", "node-one", req); err != nil {
		t.Fatal(err)
	}
	mustWith(t, b, func(st *State) {
		st.Teams[c.Team] = &Team{ID: c.Team, Repo: "example-app", Members: map[string]*TeamMember{"codex-1@mac": {Agent: "codex-1@mac", Task: c.Task, Tier: c.Tier, State: MemberActive, Branch: "agents/codex-1", Worktree: "/remote/worktree"}}}
	})
	return hubRunRegistry{bus: b, instances: registry, localInstance: "lead"}, c
}

// TestHubRunResolverTeam binds every server field to trusted metadata, never to request arguments.
func TestHubRunResolverTeam(t *testing.T) {
	registry, c := runResolverFixture(t)
	s, err := registry.resolve(c)
	if err != nil {
		t.Fatal(err)
	}
	if s.bus != registry.bus || s.me != "codex-1@mac" || s.info.Agent != s.me || s.info.Task != c.Task || s.info.Repo != "example-app" || s.info.Branch != "agents/codex-1" || s.info.Worktree != "/remote/worktree" || s.team != c.Team || s.tier != c.Tier || s.role != c.Role || s.instance != c.Instance || s.directory == nil || s.tasksDir != filepath.Join(registry.bus.dir, "remote-tasks", c.Instance) {
		t.Fatalf("incorrect trusted server: %+v", s)
	}
	// An actual hub call reaches the existing dispatcher using the qualified team member.
	hub := &Hub{Bus: registry.bus, Instances: registry.instances, Resolve: registry.resolve}
	client := &HubClient{}
	req, requestErr := client.NewRequest(c, HubToolCall, "team_status", map[string]any{})
	if requestErr != nil {
		t.Fatal(requestErr)
	}
	response, status := hub.Execute(context.Background(), "mac", req)
	if status != 200 || response.Error != nil || response.Result == nil || !strings.Contains(*response.Result, "codex-1@mac") {
		t.Fatalf("hub dispatch: %d %+v", status, response)
	}
}

// TestHubRunResolverIdentity rejects each mismatched identity and revoked assignment.
func TestHubRunResolverIdentity(t *testing.T) {
	cases := []struct {
		name   string                           // authorization mismatch exercised independently
		change func(hubRunRegistry, *HubCaller) // fixture mutation before resolving
	}{
		{"unknown-agent", func(_ hubRunRegistry, c *HubCaller) { c.Agent = "unknown" }},
		{"unknown-instance", func(_ hubRunRegistry, c *HubCaller) { c.Instance = "other" }},
		{"unknown-task", func(_ hubRunRegistry, c *HubCaller) { c.Task = "other-task" }},
		{"wrong-team", func(_ hubRunRegistry, c *HubCaller) { c.Team = "other-team" }},
		{"wrong-tier", func(_ hubRunRegistry, c *HubCaller) { c.Tier = TierWorker }},
		{"wrong-role", func(_ hubRunRegistry, c *HubCaller) { c.Role = RoleInvestigator }},
		{"qualified-caller", func(_ hubRunRegistry, c *HubCaller) { c.Agent = "codex-1@mac" }},
		{"traversal", func(_ hubRunRegistry, c *HubCaller) { c.Task = "../task-one" }},
		{"expired-instance", func(r hubRunRegistry, _ *HubCaller) {
			r.instances.Now = func() time.Time { return time.Now().Add(time.Hour) }
		}},
		{"missing-directory", func(r hubRunRegistry, _ *HubCaller) { r.instances.Bus = nil }},
		{"excluded", func(r hubRunRegistry, c *HubCaller) {
			mustWith(t, r.bus, func(st *State) { st.Teams[c.Team].Members["codex-1@mac"].State = MemberExcluded })
		}},
		{"finished-team", func(r hubRunRegistry, c *HubCaller) {
			mustWith(t, r.bus, func(st *State) { st.Teams[c.Team].Done = true })
		}},
		{"wrong-assigned-task", func(r hubRunRegistry, c *HubCaller) {
			mustWith(t, r.bus, func(st *State) { st.Teams[c.Team].Members["codex-1@mac"].Task = "other" })
		}},
		{"wrong-assigned-tier", func(r hubRunRegistry, c *HubCaller) {
			mustWith(t, r.bus, func(st *State) { st.Teams[c.Team].Members["codex-1@mac"].Tier = TierWorker })
		}},
		{"wrong-branch", func(r hubRunRegistry, c *HubCaller) {
			mustWith(t, r.bus, func(st *State) { st.Teams[c.Team].Members["codex-1@mac"].Branch = "other" })
		}},
		{"unqualified-team-member", func(r hubRunRegistry, c *HubCaller) {
			mustWith(t, r.bus, func(st *State) {
				team := st.Teams[c.Team]
				team.Members[c.Agent] = team.Members["codex-1@mac"]
				delete(team.Members, "codex-1@mac")
			})
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			r, c := runResolverFixture(t)
			tc.change(r, &c)
			s, err := r.resolve(c)
			if s != nil || err == nil || err.Cause != HubCallIdentity {
				t.Fatalf("accepted mismatched identity: %+v %v", s, err)
			}
		})
	}
}

// changeRunManifest changes an authenticated daemon report while preserving durable lease credentials.
func changeRunManifest(t *testing.T, r hubRunRegistry, change func(*HubTaskSummary)) {
	t.Helper()
	if serr := r.instances.withState(func(st *hubInstanceState) *StorageError {
		record := st.Instances["mac"]
		summary := record.Manifest.CurrentState.Tasks["task-one"]
		change(&summary)
		record.Manifest.CurrentState.Tasks["task-one"] = summary
		return r.instances.save(st)
	}); serr != nil {
		t.Fatal(serr)
	}
}

// TestHubRunResolverSoloAndQueues checks independent solo, helper and investigator authorization.
func TestHubRunResolverSoloAndQueues(t *testing.T) {
	r, c := runResolverFixture(t)
	c.Team = ""
	changeRunManifest(t, r, func(s *HubTaskSummary) { s.Team = "" })
	if _, err := r.resolve(c); err != nil {
		t.Fatal(err)
	}
	c.Tier = TierWorker
	changeRunManifest(t, r, func(s *HubTaskSummary) { s.Tier = TierWorker })
	expectRunCause(t, r, c, HubCallIdentity)
	mustWith(t, r.bus, func(st *State) {
		st.Helpers["1"] = &Helper{ID: 1, Worker: "codex-1@mac", HelperTask: c.Task, Repo: "example-app", Status: HelperRunning}
	})
	if _, err := r.resolve(c); err != nil {
		t.Fatal(err)
	}
	mustWith(t, r.bus, func(st *State) { st.Helpers["1"].Worker = "codex-1@other" })
	expectRunCause(t, r, c, HubCallIdentity)
	c.Tier = TierNormal
	c.Role = RoleInvestigator
	changeRunManifest(t, r, func(s *HubTaskSummary) { s.Tier = c.Tier; s.Role = c.Role })
	expectRunCause(t, r, c, HubCallIdentity)
	mustWith(t, r.bus, func(st *State) {
		st.Investigations["1"] = &Investigation{ID: 1, InvTask: c.Task, Investigator: c.Agent, Instance: c.Instance, Repo: "example-app", Status: InvRunning}
	})
	if _, err := r.resolve(c); err != nil {
		t.Fatal(err)
	}
	mustWith(t, r.bus, func(st *State) { st.Investigations["1"].Instance = "other" })
	expectRunCause(t, r, c, HubCallIdentity)
	mustWith(t, r.bus, func(st *State) { st.Investigations["1"].Instance = ""; st.Investigations["1"].Investigator = "" })
	expectRunCause(t, r, c, HubCallIdentity)
	mustWith(t, r.bus, func(st *State) { st.Investigations["1"].Status = InvDone })
	expectRunCause(t, r, c, HubCallIdentity)
}

// TestHubRunResolverStorage exposes registry corruption as Storage rather than an identity denial.
func TestHubRunResolverStorage(t *testing.T) {
	r, c := runResolverFixture(t)
	if err := os.WriteFile(r.bus.path("instances.json"), []byte("{"), 0o600); err != nil {
		t.Fatal(err)
	}
	expectRunCause(t, r, c, HubCallStorage)
}

// TestHubRunResolverLocalMetadata reads lead-local identity without requiring a remote manifest.
func TestHubRunResolverLocalMetadata(t *testing.T) {
	r, c := runResolverFixture(t)
	c.Instance = "lead"
	c.Team = ""
	r.tasksDir = t.TempDir()
	dir := filepath.Join(r.tasksDir, c.Task)
	if err := os.Mkdir(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	meta := "ID=task-one\nSTATUS=running\nAGENT=codex-1\nTEAM=''\nKIND=worker\nREPO_NAME=example-app\nBRANCH=agents/codex-1\nWORKTREE=/local/with\\ space\nEXTRA_ARGS=ignored\n"
	path := filepath.Join(dir, "meta.env")
	if err := os.WriteFile(path, []byte(meta), 0o600); err != nil {
		t.Fatal(err)
	}
	s, err := r.resolve(c)
	if err != nil || s.info.Worktree != "/local/with space" || s.tasksDir != r.tasksDir {
		t.Fatalf("local metadata: %+v %v", s, err)
	}
	c.Tier = TierWorker
	expectRunCause(t, r, c, HubCallIdentity)
	c.Tier = TierNormal
	if err := os.WriteFile(path, []byte(meta+"AGENT=other\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	expectRunCause(t, r, c, HubCallIdentity)
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	outside := filepath.Join(t.TempDir(), "meta.env")
	if err := os.WriteFile(outside, []byte(meta), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, path); err != nil {
		t.Fatal(err)
	}
	expectRunCause(t, r, c, HubCallIdentity)
}

// TestDecodeTaskValue covers printf-%q encodings and rejects executable shell input.
func TestDecodeTaskValue(t *testing.T) {
	cases := []struct {
		raw   string
		want  string
		valid bool
	}{ // shell word, decoded metadata and acceptance
		{"''", "", true}, {`plain/path`, "plain/path", true}, {`with\ space`, "with space", true},
		{`a\$b`, "a$b", true}, {`$'line\nnext\t\377'`, "line\nnext\t\xff", true}, {`$'\E'`, "\x1b", true},
		{"$(touch example)", "", false}, {"`id`", "", false}, {"one;two", "", false}, {"with space", "", false}, {`tail\`, "", false},
	}
	for _, tc := range cases {
		got, valid := decodeTaskValue(tc.raw)
		if valid != tc.valid || valid && got != tc.want {
			t.Errorf("decode %q: %q %v", tc.raw, got, valid)
		}
	}
}

// TestHubTaskSummaryLegacyFixture preserves pre-identity manifest decoding while denying authorization.
func TestHubTaskSummaryLegacyFixture(t *testing.T) {
	data, err := os.ReadFile("testdata/instances-before-run-identity.json")
	if err != nil {
		t.Fatal(err)
	}
	var state hubInstanceState
	if err := json.Unmarshal(data, &state); err != nil {
		t.Fatal(err)
	}
	task := state.Instances["mac"].Manifest.CurrentState.Tasks["task-one"]
	if task.TaskID != "task-one" || task.Role != "" || task.Tier != "" || task.Repo != "" {
		t.Fatalf("legacy task changed: %+v", task)
	}
}

// expectRunCause checks resolver failures using the operation's exact typed cause.
func expectRunCause(t *testing.T, r hubRunRegistry, c HubCaller, cause HubCallCause) {
	t.Helper()
	s, err := r.resolve(c)
	if s != nil {
		t.Fatal("failed resolver returned a server")
	}
	expectCause(t, err, cause)
}

// TestHubRunPeerOwnership fences node aliases before the registered server can execute a tool.
func TestHubRunPeerOwnership(t *testing.T) {
	r, c := runResolverFixture(t)
	hub := &Hub{Bus: r.bus, Instances: r.instances, Resolve: r.resolve, AuthorizePeer: r.authorizePeer}
	client := &HubClient{}
	req, err := client.NewRequest(c, HubToolCall, "team_status", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	for _, peer := range []string{"", "other-node", "local:mac"} {
		ctx := context.WithValue(context.Background(), hubPeerContextKey{}, peer)
		response, status := hub.Execute(ctx, "mac", req)
		if status != 403 || response.Error == nil || response.Error.Cause != HubCallIdentity.info().name {
			t.Fatalf("accepted alias for %q: %d %+v", peer, status, response)
		}
	}
	ctx := context.WithValue(context.Background(), hubPeerContextKey{}, "node-one")
	response, status := hub.Execute(ctx, "mac", req)
	if status != 200 || response.Error != nil {
		t.Fatalf("owner denied: %d %+v", status, response)
	}
	if err := r.authorizePeer("lead", "local:lead"); err != nil {
		t.Fatal(err)
	}
	if err := r.authorizePeer("mac", "local:lead"); err == nil || err.Cause != HubCallIdentity {
		t.Fatalf("local peer crossed instance boundary: %v", err)
	}
}

// TestHubRunLegacyInvestigationFixture loads queue records written before investigator ownership existed.
func TestHubRunLegacyInvestigationFixture(t *testing.T) {
	data, err := os.ReadFile("testdata/state-before-investigator-identity.json")
	if err != nil {
		t.Fatal(err)
	}
	b := newBus(t)
	if err := os.WriteFile(b.path("state.json"), data, 0o600); err != nil { //nolint:gosec // G703: fixture contents are copied into the fixed state filename under this test's temporary bus

		t.Fatal(err)
	}
	st, serr := b.load()
	if serr != nil {
		t.Fatal(serr)
	}
	inv := st.Investigations["1"]
	if inv == nil || inv.InvTask != "task-investigator" || inv.Investigator != "" || inv.Instance != "" {
		t.Fatalf("legacy investigation: %+v", inv)
	}
}

// TestHubServeTrustedLocalRun exercises the production resolver and peer binding through the Unix listener.
func TestHubServeTrustedLocalRun(t *testing.T) {
	home := t.TempDir()
	if err := os.Chmod(home, 0o700); err != nil { //nolint:gosec // G302: this is the private socket directory, whose owner needs execute permission

		t.Fatal(err)
	}
	busDir := filepath.Join(home, "bus")
	tasksDir := filepath.Join(home, "tasks")
	dir := filepath.Join(tasksDir, "task-one")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	meta := "ID=task-one\nSTATUS=running\nAGENT=codex-1\nTEAM=''\nKIND=worker\nTIER=normal\nREPO_NAME=example-app\nBRANCH=agents/codex-1\nWORKTREE=/local/worktree\n"
	if err := os.WriteFile(filepath.Join(dir, "meta.env"), []byte(meta), 0o600); err != nil {
		t.Fatal(err)
	}
	cert, key := generateTestCertificate(t, home)
	tcp, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	socket := filepath.Join(home, "hub.sock")
	unix, err := net.Listen("unix", socket)
	if err != nil {
		if closeErr := tcp.Close(); closeErr != nil {
			t.Error(closeErr)
		}
		t.Fatal(err)
	}
	if err := os.Chmod(socket, 0o600); err != nil {
		t.Fatal(err)
	}
	stop := make(chan struct{})
	done := make(chan *HubServeError, 1)
	go func() {
		done <- serveHub(&HubConfig{Dir: busDir, Listener: tcp, UnixListener: unix, InstanceName: "lead", TasksDir: tasksDir, Port: 7777, Auth: HubAuthModeToken, AllowedInstances: []string{"mac=127.0.0.1"}, Tokens: map[string]string{"mac": "example-token"}, TLSCertFile: cert, TLSKeyFile: key, Stop: stop})
	}()
	t.Cleanup(func() {
		close(stop)
		select {
		case err := <-done:
			if err != nil {
				t.Error(err)
			}
		case <-time.After(5 * time.Second):
			t.Error("hub did not stop")
		}
	})
	client, clientErr := newHubSocketClient(socket, "lead")
	if clientErr != nil {
		t.Fatal(clientErr)
	}
	client.RetryDuration = 2 * time.Second
	c := HubCaller{Agent: "codex-1", Instance: "lead", Task: "task-one", Role: RoleAgent, Tier: TierNormal}
	req, requestErr := client.NewRequest(c, HubToolCall, "read_messages", map[string]any{})
	if requestErr != nil {
		t.Fatal(requestErr)
	}
	response, callErr := client.Call(context.Background(), req)
	if callErr != nil || response.Result == nil {
		t.Fatalf("trusted local call: %+v %v", response, callErr)
	}
	c.Task = "unknown-task"
	req, requestErr = client.NewRequest(c, HubToolCall, "read_messages", map[string]any{})
	if requestErr != nil {
		t.Fatal(requestErr)
	}
	_, callErr = client.Call(context.Background(), req)
	var typed *HubCallError
	if !errors.As(callErr, &typed) || typed.Cause != HubCallIdentity {
		t.Fatalf("unregistered local caller: %v", callErr)
	}
}
