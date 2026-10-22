package main

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// TestStatusReadsAuthenticatedHubAcrossHomes runs the public status script against a separate TLS hub home.
func TestStatusReadsAuthenticatedHubAcrossHomes(t *testing.T) {
	hub, _ := hubViewFixture(t)
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens["mac"] = "temporary-test-token"
	auth.Token.AllowedAddrs["mac"] = "127.0.0.1"
	hub.Authenticate = auth.Authenticate
	endpoint := httptest.NewTLSServer(hub)
	t.Cleanup(endpoint.Close)
	home := t.TempDir()
	binary := filepath.Join(home, "agentbus")
	build := exec.Command("go", "build", "-o", binary, ".") //nolint:gosec // G204: fixed local Go build, stdlib-only module, output and cache in suite-owned temporary directories
	build.Env = append(os.Environ(), "GOCACHE="+filepath.Join(suiteIsolation.root, "go-cache"))
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatalf("cannot build isolated status helper: %v %s", err, output)
	}
	fingerprint := sha256.Sum256(endpoint.Certificate().Raw)
	configuration := fmt.Sprintf("declare -A AGENT_TYPE=()\nINSTANCE_NAME=mac\nLEAD_INSTANCE=%q\nHUB_AUTH=token\nHUB_CERT_SHA256=%q\nAGENTBUS_BIN=%q\n", endpoint.URL, hex.EncodeToString(fingerprint[:]), binary)
	if err := os.WriteFile(filepath.Join(home, "agents.conf"), []byte(configuration), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, "secrets.env"), []byte("HUB_TOKEN=temporary-test-token\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(home, "bus"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, "bus", "state.json"), []byte("local state must never be read"), 0o600); err != nil {
		t.Fatal(err)
	}
	script, err := filepath.Abs(filepath.Join("..", "bin", "agent"))
	if err != nil {
		t.Fatal(err)
	}
	command := exec.Command(script, "status") //nolint:gosec // G204: repository read-only status script and temporary instance configuration; no task or agent CLI is started
	command.Env = append(os.Environ(), "AGENTS_HOME="+home)
	output, err := command.CombinedOutput()
	if err != nil || !strings.Contains(string(output), "shared@mac") || !strings.Contains(string(output), "shared@linux") || !strings.Contains(string(output), "codex@mac") {
		t.Fatalf("public status did not aggregate the authenticated remote hub: %v %s", err, output)
	}
}
