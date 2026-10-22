package main

import (
	"bytes"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// instanceManifestFixture supplies a complete report without requiring a real node or tools.
func instanceManifestFixture(name string) InstanceManifest {
	return InstanceManifest{InstanceName: name, Timestamp: time.Now().UTC(), TTLSeconds: 180,
		Detected: DetectedCapabilities{OS: "linux", Arch: "arm64", Toolchains: []string{"go"}},
		Declared: []Capability{"build:go"}, CurrentState: InstanceCurrentState{Slots: SlotCapacity{Total: 2, Free: 1}}}
}

// instanceRegisterFixture is a retry-stable registration from one daemon session.
func instanceRegisterFixture(name, session string) HubInstanceRegisterRequest {
	return HubInstanceRegisterRequest{Version: hubProtocolVersion, RegistrationID: session, Sequence: 1, Manifest: instanceManifestFixture(name)}
}

// instanceHeartbeatFixture uses the exact currently issued credentials.
func instanceHeartbeatFixture(lease HubInstanceLease, sequence uint64) HubInstanceHeartbeatRequest {
	return HubInstanceHeartbeatRequest{Version: hubProtocolVersion, Instance: "mac", Token: lease.Token, Generation: lease.Generation, Sequence: sequence, Manifest: instanceManifestFixture("mac")}
}

// TestInstanceLeaseLifecycle verifies restart durability, renewal, expiration and fencing after return.
func TestInstanceLeaseLifecycle(t *testing.T) {
	now := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	registry := &HubInstanceRegistry{Bus: newBus(t), Now: func() time.Time { return now }}
	req := instanceRegisterFixture("mac", "session-one")
	lease, err := registry.Register("mac", "node-one", req)
	if err != nil {
		t.Fatal(err)
	}
	if lease.Generation != 1 || len(lease.Token) != 64 || !lease.ExpiresAt.Equal(now.Add(180*time.Second)) {
		t.Fatalf("invalid lease: %+v", lease)
	}
	again, err := registry.Register("mac", "node-one", req)
	if err != nil || again != lease {
		t.Fatalf("registration retry: %+v %v", again, err)
	}
	_, err = registry.Register("mac", "node-one", instanceRegisterFixture("mac", "session-two"))
	if err == nil || err.Cause != InstanceRegisterDuplicateInstanceName {
		t.Fatalf("duplicate daemon: %v", err)
	}
	// Another registry handle proves credentials/generations come from disk, rather than a process cache.
	restored := &HubInstanceRegistry{Bus: registry.Bus, Now: registry.Now}
	now = now.Add(time.Minute)
	heartbeat := instanceHeartbeatFixture(lease, 2)
	heartbeat.Manifest.CurrentState.Tasks = map[string]HubTaskSummary{"task-one": {TaskID: "task-one", Agent: "codex-1", Status: "running"}}
	renewed, herr := restored.Heartbeat("mac", "node-one", heartbeat)
	if herr != nil || !renewed.ExpiresAt.Equal(now.Add(180*time.Second)) {
		t.Fatalf("renewal: %+v %v", renewed, herr)
	}
	// Retrying an old registration must not revert the new report or deadline.
	replay, err := restored.Register("mac", "node-one", req)
	if err != nil || replay != renewed {
		t.Fatalf("registration rolled back lease: %+v %v", replay, err)
	}
	if serr := restored.withState(func(state *hubInstanceState) *StorageError {
		if state.Instances["mac"].Manifest.CurrentState.Tasks["task-one"].TaskID != "task-one" {
			t.Error("task summary was lost")
		}
		return nil
	}); serr != nil {
		t.Fatal(serr)
	}
	now = renewed.ExpiresAt.Time
	reachable, serr := restored.CheckReachable("mac")
	if serr != nil || reachable {
		t.Fatalf("expired instance reachable: %v %v", reachable, serr)
	}
	_, herr = restored.Heartbeat("mac", "node-one", instanceHeartbeatFixture(renewed, 3))
	if herr == nil || herr.Cause != InstanceHeartbeatLeaseExpired {
		t.Fatalf("expired heartbeat: %v", herr)
	}
	returned, err := restored.Register("mac", "node-one", instanceRegisterFixture("mac", "session-two"))
	if err != nil || returned.Generation != 2 || returned.Token == lease.Token {
		t.Fatalf("return lease: %+v %v", returned, err)
	}
	_, herr = restored.Heartbeat("mac", "node-one", instanceHeartbeatFixture(renewed, 4))
	if herr == nil || herr.Cause != InstanceHeartbeatStaleLease {
		t.Fatalf("stale daemon resurrected: %v", herr)
	}
	check, serr := restored.CheckLease("mac", "node-one", returned.Token, returned.Generation)
	if serr != nil || check != HubLeaseValid {
		t.Fatalf("new daemon not live: %v %v", check, serr)
	}
}

// TestInstanceRegisterCauses covers all registration failures without comparing error messages.
func TestInstanceRegisterCauses(t *testing.T) {
	registry := &HubInstanceRegistry{Bus: newBus(t)}
	valid := instanceRegisterFixture("mac", "session-one")
	cases := []struct {
		name   string                            // independently invalid field
		change func(*HubInstanceRegisterRequest) // mutation that must fail validation
	}{
		{"version", func(r *HubInstanceRegisterRequest) { r.Version++ }},
		{"empty session", func(r *HubInstanceRegisterRequest) { r.RegistrationID = "" }},
		{"sequence", func(r *HubInstanceRegisterRequest) { r.Sequence = 0 }},
		{"name", func(r *HubInstanceRegisterRequest) { r.Manifest.InstanceName = "../mac" }},
		{"timestamp", func(r *HubInstanceRegisterRequest) { r.Manifest.Timestamp = time.Time{} }},
		{"ttl", func(r *HubInstanceRegisterRequest) { r.Manifest.TTLSeconds = 0 }},
		{"capability", func(r *HubInstanceRegisterRequest) { r.Manifest.Declared = []Capability{"build:typo"} }},
		{"capacity", func(r *HubInstanceRegisterRequest) { r.Manifest.CurrentState.Slots.Free = 3 }},
		{"os", func(r *HubInstanceRegisterRequest) { r.Manifest.Detected.OS = "unknown" }},
		{"arch", func(r *HubInstanceRegisterRequest) { r.Manifest.Detected.Arch = "unknown" }},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			req := instanceRegisterFixture("mac", "session-one")
			tc.change(&req)
			_, err := registry.Register("mac", "node-one", req)
			if err == nil || err.Cause != InstanceRegisterMalformed {
				t.Fatalf("cause: %v", err)
			}
		})
	}
	_, err := registry.Register("mac", "node-one", instanceRegisterFixture("other", "session-one"))
	if err == nil || err.Cause != InstanceRegisterIdentity {
		t.Fatalf("identity: %v", err)
	}
	registry.Random = strings.NewReader("")
	_, err = registry.Register("mac", "node-one", valid)
	if err == nil || err.Cause != InstanceRegisterEntropy {
		t.Fatalf("entropy: %v", err)
	}
	registry.Random = nil
	if _, err := registry.Register("mac", "node-one", valid); err != nil {
		t.Fatal(err)
	}
	_, err = registry.Register("mac", "node-two", valid)
	if err == nil || err.Cause != InstanceRegisterIdentity {
		t.Fatalf("peer takeover: %v", err)
	}
	// A missing bus fails closed, preserving a typed storage error.
	_, err = (&HubInstanceRegistry{}).Register("mac", "node-one", valid)
	if err == nil || err.Cause != InstanceRegisterStorage {
		t.Fatalf("storage: %v", err)
	}
}

// TestInstanceHeartbeatCauses checks lease owner, generation, token, report ordering and malformed input.
func TestInstanceHeartbeatCauses(t *testing.T) {
	registry := &HubInstanceRegistry{Bus: newBus(t)}
	lease, err := registry.Register("mac", "node-one", instanceRegisterFixture("mac", "session"))
	if err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name   string                             // failing condition
		peer   string                             // authenticated node
		change func(*HubInstanceHeartbeatRequest) // request mutation
		cause  InstanceHeartbeatCause             // exact operation failure
	}{
		{"malformed", "node-one", func(r *HubInstanceHeartbeatRequest) { r.Version = 0 }, InstanceHeartbeatMalformed},
		{"identity", "node-one", func(r *HubInstanceHeartbeatRequest) { r.Instance = "other" }, InstanceHeartbeatIdentity},
		{"wrong peer", "node-two", func(*HubInstanceHeartbeatRequest) {}, InstanceHeartbeatIdentity},
		{"generation", "node-one", func(r *HubInstanceHeartbeatRequest) { r.Generation++ }, InstanceHeartbeatStaleLease},
		{"token", "node-one", func(r *HubInstanceHeartbeatRequest) { r.Token = "wrong" }, InstanceHeartbeatStaleLease},
		{"sequence", "node-one", func(r *HubInstanceHeartbeatRequest) { r.Sequence = 1 }, InstanceHeartbeatStaleSequence},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			req := instanceHeartbeatFixture(lease, 2)
			tc.change(&req)
			_, err := registry.Heartbeat("mac", tc.peer, req)
			if err == nil || err.Cause != tc.cause {
				t.Fatalf("cause: %v", err)
			}
		})
	}
	unknown := &HubInstanceRegistry{Bus: newBus(t)}
	_, herr := unknown.Heartbeat("mac", "node-one", instanceHeartbeatFixture(lease, 2))
	if herr == nil || herr.Cause != InstanceHeartbeatUnknownInstance {
		t.Fatalf("unknown: %v", herr)
	}
	_, herr = (&HubInstanceRegistry{}).Heartbeat("mac", "node-one", instanceHeartbeatFixture(lease, 2))
	if herr == nil || herr.Cause != InstanceHeartbeatStorage {
		t.Fatalf("storage: %v", herr)
	}
}

// TestInstanceRegistrationConcurrency proves two registries sharing one bus cannot admit two daemons.
func TestInstanceRegistrationConcurrency(t *testing.T) {
	bus := newBus(t)
	var wg sync.WaitGroup
	results := make(chan *InstanceRegisterError, 2)
	for _, session := range []string{"one", "two"} {
		wg.Add(1)
		go func(session string) {
			defer wg.Done()
			registry := &HubInstanceRegistry{Bus: bus}
			_, err := registry.Register("mac", "node-one", instanceRegisterFixture("mac", session))
			results <- err
		}(session)
	}
	wg.Wait()
	close(results)
	successes, duplicates := 0, 0
	for err := range results {
		switch {
		case err == nil:
			successes++
		case err.Cause == InstanceRegisterDuplicateInstanceName:
			duplicates++
		default:
			t.Fatal(err)
		}
	}
	if successes != 1 || duplicates != 1 {
		t.Fatalf("successes=%d duplicates=%d", successes, duplicates)
	}
}

// instanceHTTPCall uses the same authenticated TLS endpoint path a node uses, without a real CLI.
func instanceHTTPCall(t *testing.T, client *http.Client, endpoint, path, token string, body any) (int, []byte) {
	t.Helper()
	data, err := json.Marshal(body)
	if err != nil {
		t.Fatal(err)
	}
	req, err := http.NewRequest(http.MethodPost, endpoint+path, bytes.NewReader(data))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Instance-Name", "mac")
	resp, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	out, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if err := resp.Body.Close(); err != nil {
		t.Fatal(err)
	}
	return resp.StatusCode, out
}

// TestInstanceEndpointsTokenTLS covers admission, successful registration and renewal over real HTTP/TLS.
func TestInstanceEndpointsTokenTLS(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens["mac"] = "secret"
	auth.Token.AllowedAddrs["mac"] = "127.0.0.1"
	hub := &Hub{Instances: &HubInstanceRegistry{Bus: newBus(t)}, Authenticate: auth.Authenticate}
	server := httptest.NewTLSServer(hub)
	defer server.Close()
	req := instanceRegisterFixture("mac", "session")
	status, body := instanceHTTPCall(t, server.Client(), server.URL, "/v1/instances/register", "wrong", req)
	if status != http.StatusUnauthorized {
		t.Fatalf("wrong token status=%d body=%s", status, body)
	}
	status, body = instanceHTTPCall(t, server.Client(), server.URL, "/v1/instances/register", "secret", req)
	if status != http.StatusOK {
		t.Fatalf("register status=%d body=%s", status, body)
	}
	var lease HubInstanceLease
	if err := json.Unmarshal(body, &lease); err != nil {
		t.Fatal(err)
	}
	status, body = instanceHTTPCall(t, server.Client(), server.URL, "/v1/instances/heartbeat", "secret", instanceHeartbeatFixture(lease, 2))
	if status != http.StatusOK {
		t.Fatalf("heartbeat status=%d body=%s", status, body)
	}
	status, body = instanceHTTPCall(t, server.Client(), server.URL, "/v1/instances/register", "secret", instanceRegisterFixture("mac", "other-session"))
	if status != http.StatusConflict {
		t.Fatalf("duplicate status=%d body=%s", status, body)
	}
	var failure HubResponse
	if err := json.Unmarshal(body, &failure); err != nil {
		t.Fatal(err)
	}
	if failure.Error == nil || failure.Error.Op != "instance_register" || failure.Error.Cause != InstanceRegisterDuplicateInstanceName.String() {
		t.Fatalf("missing typed duplicate: %+v", failure)
	}
	for _, path := range []string{"/v1/instances/register", "/v1/instances/heartbeat"} {
		status, body = instanceHTTPCall(t, server.Client(), server.URL, path, "secret", map[string]any{"unknown": true})
		if status != http.StatusBadRequest {
			t.Fatalf("malformed %s: %d %s", path, status, body)
		}
	}
}

// TestInstanceTailscalePeerBinding uses a Unix LocalAPI stub to fence peers sharing an allowed tag.
func TestInstanceTailscalePeerBinding(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "tailscaled.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	api := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		peer := "node-one"
		if strings.HasPrefix(r.URL.Query().Get("addr"), "100.64.0.2:") {
			peer = "node-two"
		}
		if err := json.NewEncoder(w).Encode(map[string]any{"Node": map[string]any{"StableID": peer, "Name": peer + ".example.ts.net.", "Tags": []string{"tag:agents"}}}); err != nil {
			t.Error(err)
		}
	}))
	if err := api.Listener.Close(); err != nil {
		t.Fatal(err)
	}
	api.Listener = listener
	api.Start()
	defer api.Close()
	auth := NewHubAuthenticator(HubAuthModeTailscale)
	auth.Tailscale.SocketPath = socket
	auth.Tailscale.AllowedInstances = []string{"tag:agents"}
	registry := &HubInstanceRegistry{Bus: newBus(t)}
	for i, addr := range []string{"100.64.0.1:1234", "100.64.0.2:4321"} {
		data, err := json.Marshal(instanceRegisterFixture("mac", "session"))
		if err != nil {
			t.Fatal(err)
		}
		req := httptest.NewRequest(http.MethodPost, "/v1/instances/register", bytes.NewReader(data))
		req.RemoteAddr = addr
		req.Header.Set("X-Instance-Name", "mac")
		response := httptest.NewRecorder()
		(&Hub{Authenticate: auth.Authenticate, Instances: registry}).ServeHTTP(response, req)
		expected := http.StatusOK
		if i == 1 {
			expected = http.StatusForbidden
		}
		if response.Code != expected {
			t.Fatalf("peer %s status=%d body=%s", addr, response.Code, response.Body.String())
		}
	}
}

// TestInstanceRegistryCorruptionDoesNotResetLeases prevents corrupt data from allowing name reuse.
func TestInstanceRegistryCorruptionDoesNotResetLeases(t *testing.T) {
	bus := newBus(t)
	if err := os.WriteFile(bus.path("instances.json"), []byte(`{"version":1,"instances":{"mac":null}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	registry := &HubInstanceRegistry{Bus: bus}
	_, err := registry.Register("mac", "node-one", instanceRegisterFixture("mac", "session"))
	if err == nil || err.Cause != InstanceRegisterStorage {
		t.Fatalf("corrupt registry: %v", err)
	}
	if _, serr := registry.CheckReachable("mac"); serr == nil {
		t.Fatal("corruption was ignored by reachability query")
	}
}

// TestInstanceTTLConfiguration checks hub policy rather than trusting the advertised manifest TTL.
func TestInstanceTTLConfiguration(t *testing.T) {
	t.Setenv("MANIFEST_TTL", "240")
	cfg := parseHubConfig(nil)
	if cfg.ManifestTTL != 240*time.Second {
		t.Fatalf("environment TTL=%v", cfg.ManifestTTL)
	}
	cfg = parseHubConfig([]string{"--manifest-ttl", "90"})
	if cfg.ManifestTTL != 90*time.Second {
		t.Fatalf("flag TTL=%v", cfg.ManifestTTL)
	}
	for _, invalid := range []string{"bad", "-1", "0", "86401", "999999999999999999999999999999999999999"} {
		t.Setenv("MANIFEST_TTL", invalid)
		cfg = parseHubConfig(nil)
		err := serveHub(cfg)
		if err == nil || err.Cause != HubServeInvalidConfig {
			t.Fatalf("invalid TTL %q: %v", invalid, err)
		}
	}
	now := time.Date(2026, 10, 22, 16, 0, 0, 0, time.UTC)
	registry := &HubInstanceRegistry{Bus: newBus(t), TTL: 90 * time.Second, Now: func() time.Time { return now }}
	req := instanceRegisterFixture("mac", "session")
	req.Manifest.TTLSeconds = 86400
	lease, err := registry.Register("mac", "node", req)
	if err != nil || !lease.ExpiresAt.Equal(now.Add(90*time.Second)) {
		t.Fatalf("node overrode expiry: %+v %v", lease, err)
	}
}

// TestInstanceRegistryAbsentPreservesLegacyState ensures upgrading requires no registry migration.
func TestInstanceRegistryAbsentPreservesLegacyState(t *testing.T) {
	bus := newBus(t)
	legacy, err := os.ReadFile("testdata/state-legacy.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(bus.path("state.json"), legacy, 0o600); err != nil { //nolint:gosec // G703: fixed state filename in newBus(t) temporary directory; fixture bytes never select the path
		t.Fatal(err)
	}
	registry := &HubInstanceRegistry{Bus: bus}
	reachable, serr := registry.CheckReachable("mac")
	if serr != nil || reachable {
		t.Fatalf("legacy installation registry: %v %v", reachable, serr)
	}
	if _, err := registry.Register("mac", "node", instanceRegisterFixture("mac", "session")); err != nil {
		t.Fatal(err)
	}
	if _, serr := bus.load(); serr != nil {
		t.Fatalf("legacy state no longer loads: %v", serr)
	}
}
