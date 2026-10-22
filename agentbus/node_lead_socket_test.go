package main

import (
	"context"
	"net/http"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// TestLeadNodeUsesPrivateSocketWithoutNetworkCredentials covers empty/self origins and explicit/default sockets.
func TestLeadNodeUsesPrivateSocketWithoutNetworkCredentials(t *testing.T) {
	for _, explicit := range []bool{false, true} {
		for _, lead := range []string{"", "lead"} {
			t.Run(lead+"/"+map[bool]string{false: "default", true: "explicit"}[explicit], func(t *testing.T) {
				isolateHubTransportEnv(t)
				t.Setenv("HUB_AUTH", "token")
				t.Setenv("HUB_TOKEN", "must-not-send-to-local-socket")
				t.Setenv("HUB_CERT_SHA256", "")
				home := t.TempDir()
				path := filepath.Join(home, "bus", "hub.sock")
				if explicit {
					t.Setenv("HUB_SOCKET", path)
				} else {
					t.Setenv("HUB_LISTEN", "tailscale")
				}
				cfg, err := parseNodeConfig([]string{"--home", home, "--instance", "lead", "--lead", lead})
				if err != nil {
					t.Fatal(err)
				}
				client, err := newNodeClient(*cfg)
				if err != nil || client.URL != "http://local-hub" || client.Instance != "lead" || client.Token != "" {
					t.Fatalf("lead node required network credentials: %+v %v", client, err)
				}
				client.RetryDuration = time.Millisecond
				_, unavailable := client.Snapshot(context.Background())
				expectCause(t, unavailable, CmdSnapshotUnavailable)
				if _, err := os.Stat(filepath.Join(home, "bus", "state.json")); !os.IsNotExist(err) {
					t.Fatalf("missing private socket fell back to a file bus: %v", err)
				}
			})
		}
	}
}

// TestLeadSocketDoesNotOverrideAnExplicitRemoteOrigin preserves node transport when another lead is named.
func TestLeadSocketDoesNotOverrideAnExplicitRemoteOrigin(t *testing.T) {
	isolateHubTransportEnv(t)
	t.Setenv("HUB_SOCKET", filepath.Join(t.TempDir(), "ignored.sock"))
	client, err := newNodeClient(NodeConfig{Home: t.TempDir(), Instance: "mac", Lead: "http://remote.example.com:7777", Auth: HubAuthModeTailscale})
	if err != nil || client.URL != "http://remote.example.com:7777" {
		t.Fatalf("local socket overrode explicit remote lead: %+v %v", client, err)
	}
}

// TestLeadNodeRegistersAndReceivesCommandsOverPrivateSocket exercises an actual node loop with a recording executor.
func TestLeadNodeRegistersAndReceivesCommandsOverPrivateSocket(t *testing.T) {
	isolateHubTransportEnv(t)
	bus := newBus(t)
	hub := &Hub{Bus: bus, Instances: &HubInstanceRegistry{Bus: bus}, PollDuration: 5 * time.Millisecond}
	hub.Authenticate = func(r *http.Request) (string, *HubAuthError) {
		if r.Header.Get("Authorization") != "" || r.Header.Get("X-Instance-Name") != "lead" {
			t.Error("lead node lost local identity or transmitted network credentials")
		}
		setHubAuthenticatedPeer(r, "local:lead")
		return "lead", nil
	}
	t.Setenv("HUB_SOCKET", socketTestHub(t, hub))
	executor := &recordingNodeExecutor{}
	cfg := NodeConfig{Home: t.TempDir(), Instance: "lead", Auth: HubAuthModeToken, Token: "unused-network-secret", Executor: executor,
		HeartbeatInterval: 10 * time.Millisecond, RetryInterval: time.Millisecond,
		Manifest: func() (InstanceManifest, *ProcessError) { return instanceManifestFixture("lead"), nil }}
	node := &Node{Config: cfg}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan *CmdNodeError, 1)
	finished := make(chan struct{})
	go func() { defer close(finished); done <- node.Run(ctx) }()
	t.Cleanup(func() {
		cancel()
		select {
		case <-finished:
		case <-time.After(3 * time.Second):
			t.Error("lead node did not shut down")
		}
	})
	queued := false
	timer := time.NewTimer(3 * time.Second)
	defer timer.Stop()
	ticker := time.NewTicker(time.Millisecond)
	defer ticker.Stop()
	for {
		if !queued {
			reachable, serr := hub.Instances.CheckReachable("lead")
			if serr != nil {
				t.Fatal(serr)
			}
			if reachable {
				if err := hub.EnqueueCommand("lead", CommandWake, "local-task", map[string]string{"member": "codex@lead", "reason": "test"}); err != nil {
					t.Fatal(err)
				}
				queued = true
			}
		}
		executor.mu.Lock()
		delivered := len(executor.commands) == 1
		executor.mu.Unlock()
		if delivered {
			cancel()
			if err := <-done; err != nil {
				t.Fatal(err)
			}
			return
		}
		select {
		case <-timer.C:
			t.Fatal("lead node did not register and receive its local command")
		case err := <-done:
			t.Fatalf("lead node stopped before command delivery: %v", err)
		case <-ticker.C:
		}
	}
}

// TestLeadNodeRejectsRelativeSocketPaths keeps the local transport under operator-selected absolute paths.
func TestLeadNodeRejectsRelativeSocketPaths(t *testing.T) {
	isolateHubTransportEnv(t)
	t.Setenv("HUB_SOCKET", "relative.sock")
	_, err := newNodeClient(NodeConfig{Home: t.TempDir(), Instance: "lead"})
	expectCause(t, err, CmdNodeInvalidConfig)
}
