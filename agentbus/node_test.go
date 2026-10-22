package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// recordingNodeExecutor exercises dispatch and reconciliation without starting any agent CLI.
type recordingNodeExecutor struct {
	mu        sync.Mutex // protects invocation history
	commands  []Command  // executed commands
	uncertain []bool     // crash-reconciliation flags
	closed    bool       // daemon cleanup was requested
}

// Execute records the command and reports a checked synthetic outcome.
func (e *recordingNodeExecutor) Execute(_ context.Context, c Command, uncertain bool) (NodeExecutionResult, *ProcessError) {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.commands = append(e.commands, c)
	e.uncertain = append(e.uncertain, uncertain)
	return NodeExecutionResult{Outcome: CommandOutcomeDelivered}, nil
}

// Close verifies graceful cleanup without affecting any external process.
func (e *recordingNodeExecutor) Close() *ProcessError {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.closed = true
	return nil
}

// nodeTestConfig isolates every node and replaces hardware detection with a deterministic report.
func nodeTestConfig(t *testing.T, client *HubClient, executor NodeExecutor) NodeConfig {
	t.Helper()
	return NodeConfig{Home: t.TempDir(), Instance: "mac", Client: client, Executor: executor, HeartbeatInterval: 10 * time.Millisecond, RetryInterval: time.Millisecond,
		Manifest: func() (InstanceManifest, *ProcessError) { return instanceManifestFixture("mac"), nil }}
}

// TestNodeRegistrationHeartbeatAndCommandDelivery verifies live registration, heartbeats, dispatch and shutdown.
func TestNodeRegistrationHeartbeatAndCommandDelivery(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens["mac"] = "node-secret"
	auth.Token.AllowedAddrs["mac"] = "127.0.0.1"
	bus := newBus(t)
	hub := &Hub{Bus: bus, Instances: &HubInstanceRegistry{Bus: bus}, Authenticate: auth.Authenticate, PollDuration: 5 * time.Millisecond}
	heartbeats := make(chan struct{}, 20)
	acked := make(chan struct{}, 1)
	var enqueue sync.Once
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hub.ServeHTTP(w, r)
		switch r.URL.Path {
		case "/v1/instances/register":
			enqueue.Do(func() {
				if err := hub.EnqueueCommand("mac", CommandWake, "task-one", map[string]string{"member": "codex-1@mac", "reason": "message"}); err != nil {
					t.Error(err)
				}
			})
		case "/v1/instances/heartbeat":
			select {
			case heartbeats <- struct{}{}:
			default:
			}
		case "/v1/commands/ack":
			select {
			case acked <- struct{}{}:
			default:
			}
		}
	})
	server := httptest.NewTLSServer(handler)
	defer server.Close()
	executor := &recordingNodeExecutor{}
	cfg := nodeTestConfig(t, &HubClient{URL: server.URL, HTTP: server.Client(), Instance: "mac", Token: "node-secret"}, executor)
	node := &Node{Config: cfg}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan *CmdNodeError, 1)
	go func() { done <- node.Run(ctx) }()
	for _, wait := range []<-chan struct{}{acked, heartbeats} {
		select {
		case <-wait:
		case <-time.After(3 * time.Second):
			t.Fatal("node did not dispatch/heartbeat")
		}
	}
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("node did not stop")
	}
	executor.mu.Lock()
	defer executor.mu.Unlock()
	if len(executor.commands) != 1 || executor.commands[0].Kind != CommandWake || !executor.closed {
		t.Fatalf("dispatch history=%+v closed=%v", executor.commands, executor.closed)
	}
	state, serr := loadNodeState(cfg.Home, "mac")
	if serr != nil {
		t.Fatal(serr)
	}
	if state.After != 1 || state.Sequence < 2 || state.Commands["1"].Phase != nodeCommandAcknowledged {
		t.Fatalf("journal did not advance: %+v", state)
	}
}

// TestNodeCompletedJournalPreventsDuplicateExecution covers lost-ack redelivery and a changed lease generation.
func TestNodeCompletedJournalPreventsDuplicateExecution(t *testing.T) {
	executor := &recordingNodeExecutor{}
	cfg := nodeTestConfig(t, nil, executor)
	if err := os.MkdirAll(filepath.Join(cfg.Home, "node"), 0o700); err != nil {
		t.Fatal(err)
	}
	node := &Node{Config: cfg, executor: executor, state: &nodeState{Version: 1, Instance: "mac", Commands: make(map[string]*nodeCommandRecord)}}
	command := Command{Seq: 1, Generation: 1, Kind: CommandWake, Task: "task-one", Params: map[string]string{"member": "codex-1", "reason": "message"}}
	first, err := node.execute(context.Background(), command)
	if err != nil {
		t.Fatal(err)
	}
	restored, serr := loadNodeState(cfg.Home, "mac")
	if serr != nil {
		t.Fatal(serr)
	}
	node.state = restored
	command.Generation = 2
	second, err := node.execute(context.Background(), command)
	if err != nil || first != second {
		t.Fatalf("replay outcome changed: %+v %v", second, err)
	}
	executor.mu.Lock()
	count := len(executor.commands)
	executor.mu.Unlock()
	if count != 1 {
		t.Fatalf("redelivery executed %d times", count)
	}
	command.Params["reason"] = "different payload"
	_, err = node.execute(context.Background(), command)
	if err == nil || err.Cause != CmdNodeProtocol {
		t.Fatalf("sequence conflict: %v", err)
	}
}

// TestNodeCrashAcceptanceUsesReconciliation ensures accepted-but-incomplete records are never treated as new.
func TestNodeCrashAcceptanceUsesReconciliation(t *testing.T) {
	executor := &recordingNodeExecutor{}
	cfg := nodeTestConfig(t, nil, executor)
	if err := os.MkdirAll(filepath.Join(cfg.Home, "node"), 0o700); err != nil {
		t.Fatal(err)
	}
	command := Command{Seq: 1, Generation: 1, Kind: CommandWake, Task: "task-one", Params: map[string]string{"reason": "message"}}
	digest, verr := nodeCommandDigest(command)
	if verr != nil {
		t.Fatal(verr)
	}
	state := &nodeState{Version: 1, Instance: "mac", Commands: map[string]*nodeCommandRecord{"1": {Digest: digest, Phase: nodeCommandAccepted, Command: command}}}
	if serr := saveNodeState(cfg.Home, state); serr != nil {
		t.Fatal(serr)
	}
	restored, serr := loadNodeState(cfg.Home, "mac")
	if serr != nil {
		t.Fatal(serr)
	}
	node := &Node{Config: cfg, state: restored, executor: executor}
	if _, err := node.execute(context.Background(), command); err != nil {
		t.Fatal(err)
	}
	executor.mu.Lock()
	defer executor.mu.Unlock()
	if len(executor.uncertain) != 1 || !executor.uncertain[0] {
		t.Fatal("crash acceptance was blindly re-executed")
	}
}

// TestNodeTransportCauses validates strict response shapes, typed remote errors, TLS and cancellation.
func TestNodeTransportCauses(t *testing.T) {
	cases := []struct {
		name   string       // response failure being exercised
		status int          // HTTP status
		body   string       // exact synthetic hub response
		cause  CmdNodeCause // required concrete local error
	}{
		{"redirect", http.StatusFound, ``, CmdNodeProtocol},
		{"json", http.StatusOK, `not json`, CmdNodeProtocol},
		{"trailing", http.StatusOK, `{"version":1} {}`, CmdNodeProtocol},
		{"unknown field", http.StatusOK, `{"version":1,"extra":1}`, CmdNodeProtocol},
		{"unknown cause", http.StatusForbidden, `{"version":1,"error":{"op":"instance_register","cause":"Bogus","message":"rejected"}}`, CmdNodeProtocol},
		{"wrong operation", http.StatusForbidden, `{"version":1,"error":{"op":"instance_heartbeat","cause":"Identity","message":"rejected"}}`, CmdNodeProtocol},
		{"rejected", http.StatusForbidden, `{"version":1,"error":{"op":"instance_register","cause":"Identity","message":"rejected"}}`, CmdNodeRejected},
		{"storage", http.StatusServiceUnavailable, `{"version":1,"error":{"op":"instance_register","cause":"Storage","message":"unavailable"}}`, CmdNodeHubUnavailable},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(tc.status)
				if _, err := io.WriteString(w, tc.body); err != nil {
					t.Error(err)
				}
			}))
			defer server.Close()
			var out commandReply
			err := nodeExchange(context.Background(), &HubClient{URL: server.URL, Instance: "mac"}, "/v1/instances/register", map[string]int{"version": 1}, &out)
			expectCause(t, err, tc.cause)
		})
	}
	var out commandReply
	expectCause(t, nodeExchange(context.Background(), nil, "/v1/instances/register", nil, &out), CmdNodeInvalidConfig)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	expectCause(t, nodeExchange(ctx, &HubClient{URL: "http://127.0.0.1:1", Instance: "mac"}, "/v1/instances/register", nil, &out), CmdNodeHubUnavailable)
}

// TestNodeConfigAndLocalLock exercises invalid declarations, secret-safe formatting and duplicate daemon exclusion.
func TestNodeConfigAndLocalLock(t *testing.T) {
	cfg := NodeConfig{Home: t.TempDir(), Instance: "mac", Lead: "hub.example.com:7777", Token: "private-value", Auth: HubAuthModeToken}
	if strings.Contains(fmt.Sprint(cfg), cfg.Token) {
		t.Fatal("configuration formatting leaked token")
	}
	_, err := newNodeClient(cfg)
	expectCause(t, err, CmdNodeInvalidConfig)
	cfg.Declared = []Capability{"build:typo"}
	expectCause(t, validateNodeConfig(cfg), CmdNodeInvalidConfig)
	cfg.Declared = nil
	first, err := lockNodeHome(cfg.Home)
	if err != nil {
		t.Fatal(err)
	}
	defer closeHubFile(first)
	second, err := lockNodeHome(cfg.Home)
	if second != nil {
		closeHubFile(second)
		t.Fatal("duplicate node acquired lock")
	}
	expectCause(t, err, CmdNodeAlreadyRunning)
	if err := os.WriteFile(filepath.Join(cfg.Home, "node", "state.json"), []byte(`{"version":999}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadNodeState(cfg.Home, "mac"); err == nil {
		t.Fatal("corrupt journal was reset")
	}
}

// TestNodeAcknowledgmentRetriesLostResponse checks the journal survives a lost HTTP response without execution replay.
func TestNodeAcknowledgmentRetriesLostResponse(t *testing.T) {
	var mu sync.Mutex
	attempts := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req commandAckRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Error(err)
			return
		}
		if req.Seq != 1 || req.Outcome != CommandOutcomeDelivered {
			t.Error("changed acknowledgment payload")
		}
		mu.Lock()
		attempts++
		count := attempts
		mu.Unlock()
		if count == 1 {
			w.WriteHeader(http.StatusServiceUnavailable)
			if _, err := io.WriteString(w, `{"version":1,"error":{"op":"command_ack","cause":"Storage","message":"temporary"}}`); err != nil {
				t.Error(err)
			}
			return
		}
		if err := json.NewEncoder(w).Encode(commandReply{Version: 1}); err != nil {
			t.Error(err)
		}
	}))
	defer server.Close()
	cfg := nodeTestConfig(t, &HubClient{URL: server.URL, Instance: "mac"}, &recordingNodeExecutor{})
	if err := os.MkdirAll(filepath.Join(cfg.Home, "node"), 0o700); err != nil {
		t.Fatal(err)
	}
	node := &Node{Config: cfg, client: cfg.Client, renew: make(chan struct{}, 1), state: &nodeState{Version: 1, Instance: "mac", Lease: HubInstanceLease{Version: 1, Token: "lease", Generation: 1}, Commands: map[string]*nodeCommandRecord{"1": {Phase: nodeCommandCompleted, Command: Command{Seq: 1}, Result: NodeExecutionResult{Outcome: CommandOutcomeDelivered}}}}}
	err := node.acknowledge(context.Background(), Command{Seq: 1}, NodeExecutionResult{Outcome: CommandOutcomeDelivered})
	if err != nil {
		t.Fatal(err)
	}
	if node.state.After != 1 || node.state.Commands["1"].Phase != nodeCommandAcknowledged {
		t.Fatal("confirmed acknowledgment not persisted")
	}
	mu.Lock()
	defer mu.Unlock()
	if attempts != 2 {
		t.Fatalf("ack attempts=%d", attempts)
	}
}

// TestNodeLeaseLossRetainsExactRemoteError verifies concrete operation errors remain discoverable in the cause chain.
func TestNodeLeaseLossRetainsExactRemoteError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusConflict)
		if _, err := io.WriteString(w, `{"version":1,"error":{"op":"instance_heartbeat","cause":"LeaseExpired","message":"register again"}}`); err != nil {
			t.Error(err)
		}
	}))
	defer server.Close()
	var lease HubInstanceLease
	err := nodeExchange(context.Background(), &HubClient{URL: server.URL, Instance: "mac"}, "/v1/instances/heartbeat", nil, &lease)
	expectCause(t, err, CmdNodeLeaseLost)
	var remote *InstanceHeartbeatError
	if !errors.As(err, &remote) || remote.Cause != InstanceHeartbeatLeaseExpired {
		t.Fatal("lost concrete remote operation error")
	}
}

// TestNodeRequestProxyOutageAndConflictingAck distinguish availability from a terminal outcome conflict.
func TestNodeRequestProxyOutageAndConflictingAck(t *testing.T) {
	cases := []struct {
		name   string       // failure scenario
		status int          // HTTP admission status
		body   string       // proxy or typed operation reply
		cause  CmdNodeCause // exact failure to preserve
	}{
		{"proxy outage", http.StatusServiceUnavailable, "temporarily unavailable", CmdNodeHubUnavailable},
		{"conflicting ack", http.StatusConflict, `{"version":1,"error":{"op":"command_ack","cause":"ConflictingOutcome","message":"conflict"}}`, CmdNodeRejected},
		{"missing ack", http.StatusNotFound, `{"version":1,"error":{"op":"command_ack","cause":"NoSuchCommand","message":"unknown"}}`, CmdNodeRejected},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(tc.status)
				if _, err := io.WriteString(w, tc.body); err != nil {
					t.Error(err)
				}
			}))
			defer server.Close()
			var out commandReply
			expectCause(t, nodeExchange(context.Background(), &HubClient{URL: server.URL, Instance: "mac"}, "/v1/commands/ack", nil, &out), tc.cause)
		})
	}
}

// TestNodeDispatchAndStorageCauses cover daemon failures without modifying the host installation.
func TestNodeDispatchAndStorageCauses(t *testing.T) {
	node := &Node{Config: NodeConfig{Manifest: func() (InstanceManifest, *ProcessError) {
		return InstanceManifest{}, processErr("fixture", nil, "synthetic report failure")
	}}}
	_, err := node.report()
	expectCause(t, err, CmdNodeDispatch)
	home := t.TempDir()
	file := filepath.Join(home, "file")
	if err := os.WriteFile(file, []byte("not a directory"), 0o600); err != nil {
		t.Fatal(err)
	}
	node = &Node{Config: NodeConfig{Home: file, Instance: "mac", Client: &HubClient{URL: "http://hub.example.com", Instance: "mac"}}}
	expectCause(t, node.Run(context.Background()), CmdNodeStorage)
}

// TestNodePinnedTLSRejectsWrongCertificate treats pin mismatch as configuration rather than an outage.
func TestNodePinnedTLSRejectsWrongCertificate(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		if _, err := io.WriteString(w, `{"version":1}`); err != nil {
			t.Error(err)
		}
	}))
	defer server.Close()
	client, err := newNodeClient(NodeConfig{Lead: server.URL, Instance: "mac", Auth: HubAuthModeToken, Token: "secret", Fingerprint: strings.Repeat("0", 64)})
	if err != nil {
		t.Fatal(err)
	}
	var reply commandReply
	expectCause(t, nodeExchange(context.Background(), client, "/v1/commands/ack", nil, &reply), CmdNodeInvalidConfig)
	_, forwardErr := client.Send(context.Background(), HubRequest{Version: 1, RequestID: "wrong-pin", Kind: HubToolCall, Op: "send_message", Tool: "send_message"})
	expectCause(t, forwardErr, HubClientInvalidConfig)
}

// TestNodeConfigFlagsAndRetryBudget verifies the script wrapper and direct daemon share one validated vocabulary.
func TestNodeConfigFlagsAndRetryBudget(t *testing.T) {
	t.Setenv("INSTANCE_NAME", "mac")
	t.Setenv("HUB_RETRY_SECONDS", "7")
	cfg, err := parseNodeConfig([]string{"--home", t.TempDir(), "--lead", "hub.example.com:7777", "--capabilities", "build:ios,test:ios-simulator", "--accept-repos", "example-app", "--cert-sha256", strings.Repeat("0", 64), "--agent-cmd", "/usr/local/bin/agent"})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.HubRetryDuration != 7*time.Second || len(cfg.Declared) != 2 || len(cfg.AcceptRepos) != 1 || cfg.AcceptRepos[0] != "example-app" || cfg.Fingerprint != strings.Repeat("0", 64) {
		t.Fatalf("lost configuration: %+v", cfg)
	}
	t.Setenv("HUB_RETRY_SECONDS", "bad")
	_, err = parseNodeConfig([]string{"--home", t.TempDir()})
	expectCause(t, err, CmdNodeInvalidConfig)
	t.Setenv("HUB_RETRY_SECONDS", "30")
	_, err = parseNodeConfig([]string{"--home", t.TempDir(), "--capabilities", "build:typo"})
	expectCause(t, err, CmdNodeInvalidConfig)
}

// TestNodeDoesNotAcknowledgeBeforeBridgeAdmission rejects a process that exits before validating its task.
func TestNodeDoesNotAcknowledgeBeforeBridgeAdmission(t *testing.T) {
	executor := nodeExecutorFixture(t)
	// A process that exists and starts successfully still has not admitted the operation.
	executor.AgentCmd = "/bin/false"
	result, err := executor.launch(context.Background(), "task-one", CommandOutcomeStarted, "_node-start", "task-one", "new", "1")
	if err == nil || result.Outcome == CommandOutcomeStarted {
		t.Fatal("spawn success was mistaken for durable task admission")
	}
}

// TestNodeAndServeSharePreferredCertificatePin keeps the node and its per-run MCP servers on the same TLS settings.
func TestNodeAndServeSharePreferredCertificatePin(t *testing.T) {
	t.Setenv("LEAD_INSTANCE", "hub.example.com:7777")
	t.Setenv("HUB_LISTEN", "")
	t.Setenv("HUB_SOCKET", "")
	t.Setenv("HUB_AUTH", "token")
	t.Setenv("INSTANCE_NAME", "mac")
	t.Setenv("HUB_TOKEN", "synthetic-secret")
	t.Setenv("HUB_CERT_SHA256", strings.Repeat("0", 64))
	t.Setenv("HUB_CERT_FINGERPRINT", "")
	client, err := hubClientFromEnv()
	if err != nil || client == nil || client.Token != "synthetic-secret" {
		t.Fatalf("preferred pin not accepted by serve client: %v", err)
	}
}

// TestNodeHubOutageRecoversWithStableSession keeps registration retry identity stable across a proxy outage.
func TestNodeHubOutageRecoversWithStableSession(t *testing.T) {
	bus := newBus(t)
	hub := &Hub{Bus: bus, Instances: &HubInstanceRegistry{Bus: bus}, Authenticate: func(r *http.Request) (string, *HubAuthError) {
		setHubAuthenticatedPeer(r, "test:mac")
		return "mac", nil
	}, PollDuration: 5 * time.Millisecond}
	var mu sync.Mutex
	sessions := []string{}
	polled := make(chan struct{}, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/v1/instances/register" {
			data, err := io.ReadAll(r.Body)
			if err != nil {
				t.Error(err)
				return
			}
			if err := r.Body.Close(); err != nil {
				t.Error(err)
				return
			}
			var req HubInstanceRegisterRequest
			if err := json.Unmarshal(data, &req); err != nil {
				t.Error(err)
				return
			}
			mu.Lock()
			sessions = append(sessions, req.RegistrationID)
			attempt := len(sessions)
			mu.Unlock()
			if attempt == 1 {
				w.WriteHeader(http.StatusServiceUnavailable)
				if _, err := io.WriteString(w, "hub is restarting"); err != nil {
					t.Error(err)
				}
				return
			}
			r.Body = io.NopCloser(bytes.NewReader(data))
		}
		if r.URL.Path == "/v1/commands/poll" {
			select {
			case polled <- struct{}{}:
			default:
			}
		}
		hub.ServeHTTP(w, r)
	}))
	defer server.Close()
	cfg := nodeTestConfig(t, &HubClient{URL: server.URL, Instance: "mac"}, &recordingNodeExecutor{})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan *CmdNodeError, 1)
	go func() { done <- (&Node{Config: cfg}).Run(ctx) }()
	select {
	case <-polled:
	case <-time.After(3 * time.Second):
		t.Fatal("node did not recover from hub outage")
	}
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("recovered node did not shut down")
	}
	mu.Lock()
	defer mu.Unlock()
	if len(sessions) < 2 || sessions[0] == "" || sessions[0] != sessions[1] {
		t.Fatalf("session identity changed across retry: %v", sessions)
	}
}

// TestNodeEmptyOrMismatchedLeaseIsRejected prevents malformed registration results from granting authority.
func TestNodeEmptyOrMismatchedLeaseIsRejected(t *testing.T) {
	cfg := nodeTestConfig(t, nil, &recordingNodeExecutor{})
	if err := os.MkdirAll(filepath.Join(cfg.Home, "node"), 0o700); err != nil {
		t.Fatal(err)
	}
	node := &Node{Config: cfg, state: &nodeState{Version: 1, Instance: "mac", Commands: make(map[string]*nodeCommandRecord)}}
	expectCause(t, node.storeLease(HubInstanceLease{}), CmdNodeProtocol)
	node.state.Lease.Generation = 2
	expectCause(t, node.storeLease(HubInstanceLease{Version: 1, Token: "token", Generation: 1, ExpiresAt: Timestamp{Time: time.Now().Add(time.Minute)}}), CmdNodeProtocol)
}
