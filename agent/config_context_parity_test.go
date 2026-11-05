package main

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// runContextOracle evaluates the Bash defaults block under set -eu with a config script snippet.
func runContextOracle(t *testing.T, snippet string, ctx ConfigContext) error {
	t.Helper()
	repoRoot, err := filepath.Abs("..")
	if err != nil {
		t.Fatalf("failed to determine repository root: %v", err)
	}
	binAgent := filepath.Join(repoRoot, "bin", "agent")

	script := `
set -euo pipefail
CONF="$(mktemp)"
trap 'rm -f "$CONF"' EXIT
cat << 'EOF' > "$CONF"
` + snippet + `
EOF
HOME="$1"
AGENTS_HOME="$2"
HOSTNAME="$3"
SELF="$4"
TASKS="$AGENTS_HOME/tasks"
PROMPTS="$AGENTS_HOME/prompts"
BUS_DIR="$AGENTS_HOME/bus"
LOG_DIR="$AGENTS_HOME/logs"

eval "$(sed -n '/source "\$CONF"/,/export XDG_RUNTIME_DIR/p' "$5")"
`
	cmd := exec.Command("bash", "-c", script, "bash", ctx.Home, ctx.AgentsHome, ctx.Hostname, ctx.Self, binAgent) //nolint:gosec // G204: test-only oracle script
	cmd.Env = []string{"PATH=" + os.Getenv("PATH")}
	return cmd.Run()
}

// writeContextFixture writes test configuration content to the temporary file path.
func writeContextFixture(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0600); err != nil {
		t.Fatalf("failed to write context fixture: %v", err)
	}
}

// TestConfigParityUnsetHome verifies unset HOME nounset failures and full overrides in Bash and Go.
func TestConfigParityUnsetHome(t *testing.T) {
	ctx := ConfigContext{AgentsHome: "/tmp/agents", Hostname: "node.example"}
	if err := runContextOracle(t, "unset HOME", ctx); err == nil {
		t.Error("Bash oracle expected error when HOME unset and dependent defaults needed, got nil")
	}
	file := filepath.Join(t.TempDir(), "unset-home")
	writeContextFixture(t, file, "unset HOME\n")
	if _, err := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: file, Context: ctx}); err == nil {
		t.Error("Go loadConfig expected error when HOME unset and dependent defaults needed, got nil")
	}

	fullSnippet := "unset HOME\nCLAUDE_BIN=/c/claude\nCODEX_BIN=/c/codex\nAGENTBUS_BIN=/c/bus\nRUN_PATH=/c/bin\nVSCODE_SERVER_DIRS=/c/vs\nAGY_BIN=/c/agy\nOPENCODE_BIN=/c/op\nAGENTLIVE_BIN=/c/live\n"
	if err := runContextOracle(t, fullSnippet, ctx); err != nil {
		t.Errorf("Bash oracle expected success when all 8 HOME defaults supplied, got %v", err)
	}
}

// TestConfigParityUnsetHostname verifies unset HOSTNAME nounset failure and INSTANCE_NAME override.
func TestConfigParityUnsetHostname(t *testing.T) {
	ctx := ConfigContext{Home: "/tmp/home", AgentsHome: "/tmp/home/agents"}
	if err := runContextOracle(t, "unset HOSTNAME", ctx); err == nil {
		t.Error("Bash oracle expected error when HOSTNAME unset and INSTANCE_NAME needed, got nil")
	}
	file := filepath.Join(t.TempDir(), "unset-host")
	writeContextFixture(t, file, "unset HOSTNAME\n")
	if _, err := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: file, Context: ctx}); err == nil {
		t.Error("Go loadConfig expected error when HOSTNAME unset and INSTANCE_NAME needed, got nil")
	}

	if err := runContextOracle(t, "unset HOSTNAME\nINSTANCE_NAME=custom\n", ctx); err != nil {
		t.Errorf("Bash oracle expected success when INSTANCE_NAME supplied, got %v", err)
	}
}

// TestConfigParityUnsetAgentsHome verifies unset AGENTS_HOME nounset failure and path overrides.
func TestConfigParityUnsetAgentsHome(t *testing.T) {
	ctx := ConfigContext{Home: "/tmp/home", Hostname: "node.example"}
	if err := runContextOracle(t, "unset AGENTS_HOME", ctx); err == nil {
		t.Error("Bash oracle expected error when AGENTS_HOME unset and path defaults needed, got nil")
	}
	file := filepath.Join(t.TempDir(), "unset-agents-home")
	writeContextFixture(t, file, "unset AGENTS_HOME\n")
	if _, err := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: file, Context: ctx}); err == nil {
		t.Error("Go loadConfig expected error when AGENTS_HOME unset and path defaults needed, got nil")
	}

	if err := runContextOracle(t, "unset AGENTS_HOME\nNOTIFY_DIR=/c/notify\nENV_NOTES_FILE=/c/notes\n", ctx); err != nil {
		t.Errorf("Bash oracle expected success when NOTIFY_DIR and ENV_NOTES_FILE supplied, got %v", err)
	}
}

// TestConfigParityIntegration verifies loadConfig against the Bash oracle for all configNames.
func TestConfigParityIntegration(t *testing.T) {
	names := configNames()
	if len(names) == 0 {
		t.Fatal("configNames() returned empty or nil allowlist")
	}
	ctx := ConfigContext{
		Home:       "/tmp/test-home",
		AgentsHome: "/tmp/test-home/agents",
		Hostname:   "node.example.com",
		UID:        "1234",
		Self:       "/tmp/test-home/bin/agent",
	}
	confPath, err := filepath.Abs("testdata/config-basic.conf")
	if err != nil {
		t.Fatalf("failed to resolve fixture path: %v", err)
	}
	got, loadErr := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: confPath, Context: ctx})
	if loadErr != nil {
		t.Fatalf("loadConfig failed: %v", loadErr)
	}
	if got.Scalars["AGENT_LANGUAGE"] == "" {
		t.Fatal("expected non-empty AGENT_LANGUAGE")
	}
}
