package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// operatorHubFixture uses production identity resolution, peer leases and token authentication.
func operatorHubFixture(t *testing.T) (*Hub, HubCaller) {
	t.Helper()
	registry, run := runResolverFixture(t)
	// Match the production authenticator's peer key, independently of the envelope.
	if serr := registry.instances.withState(func(st *hubInstanceState) *StorageError {
		st.Instances["mac"].Peer = "token:mac"
		return registry.instances.save(st)
	}); serr != nil {
		t.Fatal(serr)
	}
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens = map[string]string{"mac": "test-token"}
	auth.Token.AllowedAddrs = map[string]string{"mac": "192.0.2.20"}
	auth.Token.RequireTLS = true
	return &Hub{Bus: registry.bus, Instances: registry.instances, Resolve: registry.resolve, AuthorizePeer: registry.authorizePeer, Authenticate: auth.Authenticate, CLI: hubCLIDispatcher(registry.bus, registry.instances)}, run
}

// operatorHTTP exercises the authenticated endpoint without opening any host listener.
func operatorHTTP(t *testing.T, h *Hub, caller HubCaller, kind HubCallKind, op string, args map[string]any) HubResponse {
	t.Helper()
	req, err := (&HubClient{}).NewRequest(caller, kind, op, args)
	if err != nil {
		t.Fatal(err)
	}
	data, jerr := json.Marshal(req)
	if jerr != nil {
		t.Fatal(jerr)
	}
	request := httptest.NewRequest(http.MethodPost, "/v1/tools/"+op, bytes.NewReader(data))
	request.RemoteAddr = "192.0.2.20:4321"
	request.TLS = &tls.ConnectionState{}
	request.Header.Set("X-Instance-Name", "mac")
	request.Header.Set("Authorization", "Bearer test-token")
	recorder := httptest.NewRecorder()
	h.ServeHTTP(recorder, request)
	var response HubResponse
	if err := json.Unmarshal(recorder.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	return response
}

// operatorCaller names the node control-plane principal, never an agent run.
func operatorCaller() HubCaller {
	return HubCaller{Agent: "cli", Task: "cli", Instance: "mac", Tier: TierNormal, Role: RoleAgent}
}

// TestHubCLIRejectsAgentAuthority reproduces the review's real-run CLI admission and executable injection.
func TestHubCLIRejectsAgentAuthority(t *testing.T) {
	h, run := operatorHubFixture(t)
	attack := `{"id":"attack","repo":"example-app","wake_cmd":"/caller/program","home":"/caller/home","members":{"victim":{"agent":"victim","task":"/caller/file","state":"sleeping","tier":"normal"}}}`
	response := operatorHTTP(t, h, run, HubCLICall, "team-create", map[string]any{"team_json": attack})
	if response.Error == nil || response.Error.Op != "hub" || response.Error.Cause != HubCallIdentity.String() {
		t.Fatalf("run entered operator dispatcher: %+v", response)
	}
	// Even a node operator cannot give the hub an executable or an unqualified local wake target.
	response = operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-create", map[string]any{"team_json": attack})
	if response.Error == nil || response.Error.Cause != CmdTeamCreateInvalidJSON.String() {
		t.Fatalf("executable fields admitted: %+v", response)
	}
	attack = `{"id":"attack","repo":"example-app","members":{"victim":{"agent":"victim","task":"task","state":"sleeping","tier":"normal"}}}`
	response = operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-create", map[string]any{"team_json": attack})
	if response.Error == nil || response.Error.Cause != CmdTeamCreateMissingFields.String() {
		t.Fatalf("unqualified local wake admitted: %+v", response)
	}
	mustWith(t, h.Bus, func(st *State) {
		if st.Teams["attack"] != nil {
			t.Error("rejected team stored")
		}
	})
	response = operatorHTTP(t, h, operatorCaller(), HubToolCall, "team_status", nil)
	if response.Error == nil || response.Error.Cause != HubCallIdentity.String() {
		t.Fatalf("operator impersonated agent: %+v", response)
	}
}

// TestHubCLIPublicProjection omits credentials, quiz answers and paths and limits team access by node.
func TestHubCLIPublicProjection(t *testing.T) {
	h, run := operatorHubFixture(t)
	response := operatorHTTP(t, h, operatorCaller(), HubCLICall, "instances", nil)
	if response.Error != nil || response.Stdout == nil || !strings.HasPrefix(string(*response.Stdout), "mac\t") {
		t.Fatalf("operator rejected or output changed: %+v", response)
	}
	if strings.Contains(string(*response.Stdout), "token") || strings.Contains(string(*response.Stdout), "session") || strings.Contains(string(*response.Stdout), "lease") {
		t.Fatal("private registry material exposed")
	}
	mustWith(t, h.Bus, func(st *State) {
		st.Teams[run.Team].WakeCmd = "/private/program"
		st.Teams[run.Team].Home = "/private/home"
		st.Teams[run.Team].Quiz = &Quiz{}
		st.Teams["other-team"] = &Team{ID: "other-team", Members: map[string]*TeamMember{"other@other": {Agent: "other@other", State: MemberActive, Tier: TierNormal}}}
	})
	response = operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-get", map[string]any{"team": run.Team})
	if response.Error != nil || response.Stdout == nil {
		t.Fatalf("owned team inaccessible: %+v", response)
	}
	data := string(*response.Stdout)
	if strings.Contains(data, "quiz") || strings.Contains(data, "/private/") || strings.Contains(data, "/remote/worktree") {
		t.Fatalf("private team material: %s", data)
	}
	response = operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-get", map[string]any{"team": "other-team"})
	if response.Error == nil || response.Error.Cause != CmdTeamGetNoSuchTeam.String() {
		t.Fatalf("other team exposed: %+v", response)
	}
}

// TestHubCLISharedTeamTransitions preserves announcements and exclusion through the production adapter.
func TestHubCLISharedTeamTransitions(t *testing.T) {
	h, _ := operatorHubFixture(t)
	team := &Team{ID: "new-team", Repo: "example-app", WakeCmd: "/local/program", Home: "/local/home", Members: map[string]*TeamMember{"new@mac": {Agent: "new@mac", Task: "new-task", State: MemberExcluded, Tier: TierNormal}}}
	raw, err := remoteTeamJSON(team)
	if err != nil {
		t.Fatal(err)
	}
	response := operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-create", map[string]any{"team_json": raw})
	if response.Error != nil {
		t.Fatalf("create: %+v", response)
	}
	response = operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-set", map[string]any{"team": "new-team", "agent": "new", "state": string(MemberActive)})
	if response.Error != nil {
		t.Fatalf("set: %+v", response)
	}
	mustWith(t, h.Bus, func(st *State) {
		saved := st.Teams[team.ID]
		if saved.Created.IsZero() || saved.WakeCmd != "" || saved.Home != "" || saved.Members["new@mac"].State != MemberExcluded {
			t.Fatalf("lost create/set contracts: %+v", saved)
		}
	})
	for range 2 {
		response = operatorHTTP(t, h, operatorCaller(), HubCLICall, "team-done", map[string]any{"team": team.ID})
		if response.Error != nil {
			t.Fatalf("done: %+v", response)
		}
	}
	messages, serr := h.Bus.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	if len(messages) != 2 {
		t.Fatalf("expected create and one done announcement, got %+v", messages)
	}
}

// TestHubCLIRemoteCollisionAndResume keeps private resume state on the hub and prevents replacement.
func TestHubCLIRemoteCollisionAndResume(t *testing.T) {
	h, run := operatorHubFixture(t)
	mustWith(t, h.Bus, func(st *State) { st.Teams[run.Team].Quiz = &Quiz{} })
	spec := remoteTeamSpec{ID: run.Team, Repo: "example-app", Members: map[string]*TeamMember{"new@mac": {Agent: "new@mac", Task: "new-task", Tier: TierNormal, State: MemberActive}}}
	_, err := spec.team("mac")
	if err != nil {
		t.Fatal(err)
	}
	team, err := spec.team("mac")
	if err != nil {
		t.Fatal(err)
	}
	expectCause(t, createRemoteTeamRecord(h.Bus, team, "mac"), CmdTeamCreateAlreadyExists)
	team.ID, team.Resumed = "resumed-team", run.Team
	if err := createRemoteTeamRecord(h.Bus, team, "mac"); err != nil {
		t.Fatal(err)
	}
	mustWith(t, h.Bus, func(st *State) {
		if st.Teams[team.ID].Quiz == nil || st.Teams[run.Team].Quiz == nil {
			t.Fatal("resume discarded qualification")
		}
	})
}

// TestForwardCLIPropagatesOutputFailure preserves the actual command's Storage cause.
func TestForwardCLIPropagatesOutputFailure(t *testing.T) {
	h, _ := operatorHubFixture(t)
	service := httptest.NewTLSServer(h)
	defer service.Close()
	// Use the real transport factory's pinned TLS settings.
	t.Setenv("LEAD_INSTANCE", service.URL)
	t.Setenv("HUB_LISTEN", "")
	t.Setenv("HUB_SOCKET", "")
	t.Setenv("INSTANCE_NAME", "mac")
	t.Setenv("HUB_AUTH", "token")
	t.Setenv("HUB_TOKEN", "test-token")
	t.Setenv("HUB_CERT_SHA256", fmt.Sprintf("%x", sha256.Sum256(service.Certificate().Raw)))
	// The httptest address is authenticated and owned independently of the request identity.
	h.Authenticate = func(r *http.Request) (string, *HubAuthError) {
		setHubAuthenticatedPeer(r, "token:mac")
		return "mac", nil
	}
	file, err := os.CreateTemp(t.TempDir(), "closed")
	if err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	previous := os.Stdout
	os.Stdout = file
	t.Cleanup(func() { os.Stdout = previous })
	forwarded, callErr := forwardCLI(h.Bus.dir, "instances", nil)
	var operation *CmdInstancesError
	if !forwarded || !errors.As(callErr, &operation) || operation.Cause != CmdInstancesStorage {
		t.Fatalf("stdout error lost: %v %v", forwarded, callErr)
	}
}

// TestCommandEnqueueUnavailableForwarding never writes a local queue while the hub is down.
func TestCommandEnqueueUnavailableForwarding(t *testing.T) {
	b := newBus(t)
	t.Setenv("LEAD_INSTANCE", "")
	t.Setenv("HUB_LISTEN", "tailscale")
	t.Setenv("HUB_SOCKET", filepath.Join(t.TempDir(), "missing.sock"))
	t.Setenv("INSTANCE_NAME", "mac")
	t.Setenv("HUB_RETRY_SECONDS", "1")
	forwarded, err := forwardCLI(b.dir, "command_enqueue", map[string]any{"instance": "mac", "kind": "wake", "task": "task-one"})
	var operation *CommandEnqueueError
	if !forwarded || !errors.As(err, &operation) || operation.Cause != CommandEnqueueUnavailable {
		t.Fatalf("outage cause: %v %v", forwarded, err)
	}
	if _, err := os.Stat(filepath.Join(b.dir, "commands.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("local queue accessed: %v", err)
	}
}

// TestHubCLIServePrivateSocket installs the production CLI dispatcher on the lead's owner-only listener.
func TestHubCLIServePrivateSocket(t *testing.T) {
	dir, dirErr := os.MkdirTemp(t.TempDir(), "private-")
	if dirErr != nil {
		t.Fatal(dirErr)
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	stop := make(chan struct{})
	done := make(chan *HubServeError, 1)
	socket := filepath.Join(dir, "hub.sock")
	cfg := &HubConfig{Dir: dir, Listener: listener, SocketPath: socket, InstanceName: "lead", Port: 7777, Auth: HubAuthModeTailscale, AllowedInstances: []string{"lead"}, Stop: stop}
	go func() { done <- serveHub(cfg) }()
	t.Cleanup(func() {
		close(stop)
		select {
		case err := <-done:
			if err != nil {
				t.Errorf("shutdown: %v", err)
			}
		case <-time.After(5 * time.Second):
			t.Error("hub shutdown timed out")
		}
	})
	client, clientErr := newHubSocketClient(socket, "lead")
	if clientErr != nil {
		t.Fatal(clientErr)
	}
	client.RetryDuration = 2 * time.Second
	caller := operatorCaller()
	caller.Instance = "lead"
	req, reqErr := client.NewRequest(caller, HubCLICall, "instances", nil)
	if reqErr != nil {
		t.Fatal(reqErr)
	}
	response, callErr := client.Call(context.Background(), req)
	if callErr != nil || response.Stdout == nil {
		info, statErr := os.Stat(socket)
		t.Fatalf("private CLI dispatcher: %+v %v socket %s info %v stat %v", response, callErr, socket, info, statErr)
	}
}
