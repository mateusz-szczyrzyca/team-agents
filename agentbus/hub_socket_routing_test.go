package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// isolateHubTransportEnv keeps these tests independent of the running installation's configuration.
func isolateHubTransportEnv(t *testing.T) {
	t.Helper()
	for _, key := range []string{"LEAD_INSTANCE", "INSTANCE_NAME", "HUB_LISTEN", "HUB_SOCKET", "HUB_AUTH", "HUB_TOKEN", "HUB_CERT_FINGERPRINT", "HUB_RETRY_SECONDS"} {
		t.Setenv(key, "")
	}
}

func TestLeadSocketRoutingUsesConfiguredDirectoryOrExplicitPath(t *testing.T) {
	for _, explicit := range []bool{false, true} {
		name := "default path from serve directory"
		if explicit {
			name = "explicit socket without listener setting"
		}
		t.Run(name, func(t *testing.T) {
			isolateHubTransportEnv(t)
			h, req := testHub(t)
			path := socketTestHub(t, h)
			dir := filepath.Dir(path)
			if explicit {
				t.Setenv("HUB_SOCKET", path)
				dir = filepath.Join(t.TempDir(), "unused")
			} else {
				t.Setenv("HUB_LISTEN", "127.0.0.1:7777")
			}
			t.Setenv("INSTANCE_NAME", "mac")
			// Local socket authentication must not require or transmit a network token.
			t.Setenv("HUB_AUTH", "token")
			t.Setenv("HUB_TOKEN", "unused-network-token")
			t.Setenv("LEAD_INSTANCE", "https://unused.example.com")
			t.Setenv("HUB_RETRY_SECONDS", "7")
			client, err := hubClientForDir(dir)
			if err != nil || client == nil || client.Token != "" || client.RetryDuration != 7*time.Second {
				t.Fatalf("socket configuration: %#v %v", client, err)
			}
			var out bytes.Buffer
			s := &Server{bus: &Bus{dir: dir}, out: json.NewEncoder(&out), hubCaller: req.Caller, hubClient: client}
			s.handle([]byte(`{"id":1,"method":"tools/call","params":{"name":"set_status","arguments":{"status":"lead-local"}}}`))
			if !strings.Contains(out.String(), "Status set.") || strings.Contains(out.String(), `"isError":true`) {
				t.Fatalf("socket routing did not reach the hub: %s", out.String())
			}
			if _, err := os.Stat(filepath.Join(dir, "state.json")); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("lead-local serve touched bus files: %v", err)
			}
		})
	}
}

func TestLeadSocketRoutingMissingSocketNeverFallsBack(t *testing.T) {
	for _, explicit := range []bool{false, true} {
		name := "listener configured"
		if explicit {
			name = "explicit socket configured"
		}
		t.Run(name, func(t *testing.T) {
			isolateHubTransportEnv(t)
			dir := filepath.Join(t.TempDir(), "bus-must-not-exist")
			if explicit {
				t.Setenv("HUB_SOCKET", filepath.Join(dir, "hub.sock"))
			} else {
				t.Setenv("HUB_LISTEN", "127.0.0.1:7777")
			}
			t.Setenv("INSTANCE_NAME", "mac")
			client, err := hubClientForDir(dir)
			if client == nil || err != nil {
				t.Fatalf("missing socket silently selected local mode: %#v %v", client, err)
			}
			client.RetryDuration = 10 * time.Millisecond
			_, req := testHub(t)
			var out bytes.Buffer
			s := &Server{bus: &Bus{dir: dir}, out: json.NewEncoder(&out), hubCaller: req.Caller, hubClient: client}
			_, _, callErr := s.callForMCP("send_message", map[string]any{"to": "reviewer", "text": "ready"})
			var typed *SendMessageError
			if !errors.As(callErr, &typed) || typed.Cause != SendMessageUnavailable || !errors.Is(callErr, ErrUnavailable) {
				t.Fatalf("missing socket did not return typed Unavailable: %v", callErr)
			}
			if _, err := os.Stat(dir); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("missing socket created local bus state: %v", err)
			}
		})
	}
}

func TestLeadSocketRoutingNeedsLocalIdentity(t *testing.T) {
	isolateHubTransportEnv(t)
	t.Setenv("HUB_LISTEN", "127.0.0.1:7777")
	_, err := hubClientForDir(t.TempDir())
	failsWith(t, HubClientInvalidConfig)("", err)
}
