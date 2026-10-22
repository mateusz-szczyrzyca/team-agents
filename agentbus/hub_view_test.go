package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// hubViewFixture registers two separate node reports with a colliding raw task ID.
func hubViewFixture(t *testing.T) (*Hub, []string) {
	t.Helper()
	bus := newBus(t)
	hub := &Hub{Bus: bus, Instances: &HubInstanceRegistry{Bus: bus}}
	var credentials []string
	for _, name := range []string{"mac", "linux"} {
		request := instanceRegisterFixture(name, "private-session-"+name)
		request.Manifest.CurrentState.Tasks = map[string]HubTaskSummary{"shared": {TaskID: "shared", Agent: "codex", Repo: "example-app", Status: nodeTaskRunning}}
		lease, err := hub.Instances.Register(name, "token:"+name, request)
		if err != nil {
			t.Fatal(err)
		}
		credentials = append(credentials, lease.Token, request.RegistrationID)
	}
	createTeam(t, bus, "ViewTeam", map[string][3]string{"codex@mac": {"normal", "a", "active"}})
	mustWith(t, bus, func(st *State) {
		st.Teams["ViewTeam"].Home = "/private/operator-home"
		st.Teams["ViewTeam"].WakeCmd = "/private/operator-wake"
		st.Teams["ViewTeam"].Quiz = &Quiz{Answers: map[string]*QuizAnswer{"codex@mac": {Answer: "private-quiz-answer"}}}
	})
	return hub, credentials
}

// TestHubViewTLSAndSocketAggregateWithoutCredentials covers both supported view transports and projections.
func TestHubViewTLSAndSocketAggregateWithoutCredentials(t *testing.T) {
	hub, credentials := hubViewFixture(t)
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens["mac"] = "test-network-token"
	auth.Token.AllowedAddrs["mac"] = "127.0.0.1"
	hub.Authenticate = auth.Authenticate
	endpoint := httptest.NewTLSServer(hub)
	t.Cleanup(endpoint.Close)
	client := &HubClient{URL: endpoint.URL, Instance: "mac", Token: "test-network-token", HTTP: endpoint.Client()}
	view, err := client.Snapshot(context.Background())
	if err != nil || len(view.Instances) != 2 || len(view.Instances[0].Tasks) != 1 || view.Instances[0].Name != "linux" {
		t.Fatalf("remote overview did not aggregate reports: %+v %v", view, err)
	}
	data, jerr := json.Marshal(view)
	if jerr != nil {
		t.Fatal(jerr)
	}
	credentials = append(credentials, "test-network-token", "private-quiz-answer", "/private/operator-home", "/private/operator-wake")
	for _, private := range credentials {
		if strings.Contains(string(data), private) {
			t.Fatalf("view exposed private credential/dispatch data: %q", private)
		}
	}
	// Owner-only local sockets use the same handler and projection without network credentials.
	hub.Authenticate = func(*http.Request) (string, *HubAuthError) { return "mac", nil }
	socket, failed := newHubSocketClient(socketTestHub(t, hub), "mac")
	if failed != nil {
		t.Fatal(failed)
	}
	local, err := socket.Snapshot(context.Background())
	if err != nil || len(local.Instances) != 2 {
		t.Fatalf("lead-local view differs: %+v %v", local, err)
	}
}

// TestHubViewEndpointCauses exercises each endpoint cause and strict request parsing after authentication.
func TestHubViewEndpointCauses(t *testing.T) {
	hub, _ := hubViewFixture(t)
	hub.Authenticate = func(*http.Request) (string, *HubAuthError) { return "mac", nil }
	for _, tc := range []struct {
		method string       // request method
		body   string       // exact input bytes
		cause  HubViewCause // endpoint failure
	}{
		{http.MethodGet, `{}`, HubViewMalformed},
		{http.MethodPost, `invalid`, HubViewMalformed},
		{http.MethodPost, `{"version":2,"instance":"mac"}`, HubViewMalformed},
		{http.MethodPost, `{"version":1,"instance":"mac","extra":1}`, HubViewMalformed},
		{http.MethodPost, `{"version":1,"instance":"mac"}{}`, HubViewMalformed},
		{http.MethodPost, `{"version":1,"instance":"linux"}`, HubViewIdentity},
	} {
		reply := httptest.NewRecorder()
		hub.ServeHTTP(reply, httptest.NewRequest(tc.method, "/v1/view", strings.NewReader(tc.body)))
		var response hubViewResponse
		if err := json.Unmarshal(reply.Body.Bytes(), &response); err != nil {
			t.Fatal(err)
		}
		if response.Error == nil {
			t.Fatalf("invalid request unexpectedly succeeded: %s", tc.body)
		}
		remote, known := wireOperationError(response.Error.Op, response.Error.Cause, response.Error.Message)
		var exact *HubViewError
		if !known || !errors.As(remote, &exact) || exact.Cause != tc.cause {
			t.Fatalf("view request lost cause: %s %v", tc.body, remote)
		}
	}
	hub.Instances = nil
	_, err := hub.viewSnapshot()
	expectCause(t, err, HubViewStorage)
}

// TestSnapshotClientCauses checks config/protocol/permission/outage failures without a local-state fallback.
func TestSnapshotClientCauses(t *testing.T) {
	_, invalid := (&HubClient{URL: "ftp://example.com", Instance: "mac"}).Snapshot(context.Background())
	expectCause(t, invalid, CmdSnapshotInvalidConfig)
	for _, tc := range []struct {
		body  string           // server fixture
		cause CmdSnapshotCause // client operation cause
	}{
		{`{"version":2}`, CmdSnapshotProtocol},
		{`{"version":1,"error":{"op":"hub_view","cause":"Unknown","message":"invalid"}}`, CmdSnapshotProtocol},
		{`{"version":1,"error":{"op":"hub_auth","cause":"WrongToken","message":"denied"}}`, CmdSnapshotRejected},
	} {
		endpoint := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			if _, err := w.Write([]byte(tc.body)); err != nil {
				t.Error(err)
			}
		}))
		client := &HubClient{URL: endpoint.URL, Instance: "mac", HTTP: endpoint.Client(), RetryDuration: time.Millisecond}
		_, err := client.Snapshot(context.Background())
		endpoint.Close()
		expectCause(t, err, tc.cause)
		if tc.cause == CmdSnapshotRejected {
			var remote *HubAuthError
			if !errors.As(err, &remote) || remote.Cause != HubAuthWrongToken {
				t.Fatal("snapshot client lost its typed remote authentication cause")
			}
		}
	}
	endpoint := httptest.NewServer(http.NotFoundHandler())
	client := &HubClient{URL: endpoint.URL, Instance: "mac", HTTP: endpoint.Client(), RetryDuration: time.Millisecond}
	endpoint.Close()
	_, down := client.Snapshot(context.Background())
	expectCause(t, down, CmdSnapshotUnavailable)
}

// TestSnapshotCLIStorageAndTimeout preserves operation errors for bad local storage and stdout.
func TestSnapshotCLIStorageAndTimeout(t *testing.T) {
	isolateHubTransportEnv(t)
	expectCause(t, runSnapshot([]string{"--timeout", "0"}), CmdSnapshotInvalidConfig)
	root := t.TempDir()
	file := filepath.Join(root, "file")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	expectCause(t, runSnapshot([]string{"--dir", file}), CmdSnapshotStorage)
	stdout, err := os.Open(file) //nolint:gosec // G304: read-only stdout failure fixture under t.TempDir, not a live home path
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := stdout.Close(); err != nil {
			t.Error(err)
		}
	})
	old := os.Stdout
	os.Stdout = stdout // deliberately read-only, so JSON output fails
	t.Cleanup(func() { os.Stdout = old })
	expectCause(t, runSnapshot([]string{"--dir", t.TempDir()}), CmdSnapshotStorage)
	os.Stdout = old
}

// TestHubViewKeepsPrivateAnswersOutOfJSON guards all projection fields rather than a struct's memory layout.
func TestHubViewKeepsPrivateAnswersOutOfJSON(t *testing.T) {
	hub, _ := hubViewFixture(t)
	hub.Authenticate = func(*http.Request) (string, *HubAuthError) { return "mac", nil }
	reply := httptest.NewRecorder()
	hub.ServeHTTP(reply, httptest.NewRequest(http.MethodPost, "/v1/view", bytes.NewBufferString(`{"version":1,"instance":"mac"}`)))
	if reply.Code != http.StatusOK || strings.Contains(reply.Body.String(), "lease_token") || strings.Contains(reply.Body.String(), "wake_cmd") || strings.Contains(reply.Body.String(), "private-quiz-answer") {
		t.Fatalf("unsafe hub view: %s", reply.Body.String())
	}
}
