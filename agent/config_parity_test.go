package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// runBashOracle evaluates an agents.conf fixture through the isolated Bash default block from bin/agent.
func runBashOracle(t *testing.T, confPath string, ctx ConfigContext) map[string]string {
	t.Helper()
	repoRoot, err := filepath.Abs("..")
	if err != nil {
		t.Fatalf("failed to determine repository root: %v", err)
	}
	binAgent := filepath.Join(repoRoot, "bin", "agent")

	script := `
set -e
CONF="$1"
HOME="$2"
AGENTS_HOME="$3"
HOSTNAME="$4"
SELF="$5"
TASKS="$AGENTS_HOME/tasks"
PROMPTS="$AGENTS_HOME/prompts"
BUS_DIR="$AGENTS_HOME/bus"
LOG_DIR="$AGENTS_HOME/logs"

eval "$(sed -n '/source "\$CONF"/,/export XDG_RUNTIME_DIR/p' "$6")"

for var in CLAUDE_BIN CODEX_BIN AGENTBUS_BIN RUN_PATH NOTIFY_DIR ENV_NOTES_FILE \
           AGENT_WEB_LISTEN INSTANCE_NAME AGENT_LANGUAGE AGENT_HANG_MIN \
           AGENT_NETWORK_MAX_PAUSES AGENT_RESTART_MAX AGENT_REVIEW_DRAW \
           ROUND_MAX_LINES ROUND_BOUNCES CLAUDE_TOOL_SEARCH AUTH_MODEL_OPENAI \
           AUTH_OPENROUTER_URL INV_SLOTS NOTIFY_SCRIPT ROUND_ALWAYS_EXCLUDE \
           REVIEW_HISTORY_MAX NOTIFY_CMD AUTO_MERGE_HUMAN_PATHS QUESTIONS_EDITOR; do
    if declare -p "$var" &>/dev/null; then
        printf "%s=%s\n" "$var" "${!var}"
    fi
done
if [[ ${INVESTIGATORS_TYPE[default]:-} != "" ]]; then
    printf "INVESTIGATORS_TYPE_DEFAULT=%s\n" "${INVESTIGATORS_TYPE[default]}"
elif [[ ${INVESTIGATORS_TYPE[investigator]:-} != "" ]]; then
    printf "INVESTIGATORS_TYPE_INVESTIGATOR=%s\n" "${INVESTIGATORS_TYPE[investigator]}"
fi
`
	cmd := exec.Command("bash", "-c", script, "bash", confPath, ctx.Home, ctx.AgentsHome, ctx.Hostname, ctx.Self, binAgent) //nolint:gosec // G204: fixed test-only Bash invocation evaluating local fixture paths.
	cmd.Env = []string{"PATH=" + os.Getenv("PATH")}
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("bash oracle execution failed: %v", err)
	}

	result := make(map[string]string)
	for _, line := range strings.Split(string(out), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			result[parts[0]] = parts[1]
		}
	}
	return result
}

// TestConfigParityGoldenDefaults verifies Go defaults match the immutable Bash defaults block.
func TestConfigParityGoldenDefaults(t *testing.T) {
	ctx := ConfigContext{
		Home:       "/tmp/test-parity-home",
		AgentsHome: "/tmp/test-parity-home/agents",
		Hostname:   "parity.example.com",
		UID:        "1234",
		Self:       "/tmp/bin/agent",
	}

	oracle := runBashOracle(t, os.DevNull, ctx)
	got := applyConfigDefaults(newConfig(), ctx)

	checkedKeys := []string{
		"CLAUDE_BIN", "CODEX_BIN", "RUN_PATH", "NOTIFY_DIR", "ENV_NOTES_FILE",
		"AGENT_WEB_LISTEN", "INSTANCE_NAME", "AGENT_LANGUAGE", "AGENT_HANG_MIN",
		"AGENT_NETWORK_MAX_PAUSES", "AGENT_RESTART_MAX", "AGENT_REVIEW_DRAW",
		"ROUND_MAX_LINES", "ROUND_BOUNCES", "CLAUDE_TOOL_SEARCH", "AUTH_MODEL_OPENAI",
		"AUTH_OPENROUTER_URL", "INV_SLOTS", "NOTIFY_SCRIPT", "ROUND_ALWAYS_EXCLUDE",
		"REVIEW_HISTORY_MAX",
	}

	for _, key := range checkedKeys {
		expected, existsInOracle := oracle[key]
		if !existsInOracle {
			t.Errorf("key %s missing from Bash oracle dump", key)
			continue
		}
		actual, existsInGo := got.Scalars[key]
		if !existsInGo || actual != expected {
			t.Errorf("parity mismatch for %s: Go got %q (present=%t), Bash oracle has %q", key, actual, existsInGo, expected)
		}
	}

	if oracleDefault, ok := oracle["INVESTIGATORS_TYPE_INVESTIGATOR"]; ok {
		if got.Associative["INVESTIGATORS_TYPE"]["investigator"] != oracleDefault {
			t.Errorf("INVESTIGATORS_TYPE[investigator]: Go got %q, Bash oracle has %q",
				got.Associative["INVESTIGATORS_TYPE"]["investigator"], oracleDefault)
		}
	}
}

// TestConfigParityCustomBasic verifies custom settings override defaults identically.
func TestConfigParityCustomBasic(t *testing.T) {
	confPath, err := filepath.Abs("testdata/config-basic.conf")
	if err != nil {
		t.Fatalf("failed to resolve fixture path: %v", err)
	}

	ctx := ConfigContext{
		Home:       "/tmp/test-parity-home",
		AgentsHome: "/tmp/test-parity-home/agents",
		Hostname:   "parity.example.com",
		UID:        "1234",
		Self:       "/tmp/bin/agent",
	}

	oracle := runBashOracle(t, confPath, ctx)

	// Simulating input loaded with custom overrides
	input := newConfig()
	input.Scalars["CLAUDE_BIN"] = "/custom/bin/claude"
	input.Scalars["AGENT_HANG_MIN"] = "15"

	got := applyConfigDefaults(input, ctx)

	if got.Scalars["CLAUDE_BIN"] != oracle["CLAUDE_BIN"] {
		t.Errorf("CLAUDE_BIN: Go %q != Bash %q", got.Scalars["CLAUDE_BIN"], oracle["CLAUDE_BIN"])
	}
	if got.Scalars["AGENT_HANG_MIN"] != oracle["AGENT_HANG_MIN"] {
		t.Errorf("AGENT_HANG_MIN: Go %q != Bash %q", got.Scalars["AGENT_HANG_MIN"], oracle["AGENT_HANG_MIN"])
	}
}

// TestConfigParityEmptyVsUnset verifies explicitly empty strings vs unset behavior.
func TestConfigParityEmptyVsUnset(t *testing.T) {
	confPath, err := filepath.Abs("testdata/config-empty-unset.conf")
	if err != nil {
		t.Fatalf("failed to resolve fixture path: %v", err)
	}

	ctx := ConfigContext{
		Home:       "/tmp/test-parity-home",
		AgentsHome: "/tmp/test-parity-home/agents",
		Hostname:   "parity.example.com",
		UID:        "1234",
		Self:       "/tmp/bin/agent",
	}

	oracle := runBashOracle(t, confPath, ctx)

	input := newConfig()
	input.Scalars["NOTIFY_CMD"] = ""
	input.Scalars["AUTO_MERGE_HUMAN_PATHS"] = ""
	input.Scalars["QUESTIONS_EDITOR"] = ""

	got := applyConfigDefaults(input, ctx)

	if got.Scalars["NOTIFY_CMD"] != oracle["NOTIFY_CMD"] {
		t.Errorf("NOTIFY_CMD: Go %q != Bash %q", got.Scalars["NOTIFY_CMD"], oracle["NOTIFY_CMD"])
	}
	if got.Scalars["AUTO_MERGE_HUMAN_PATHS"] != oracle["AUTO_MERGE_HUMAN_PATHS"] {
		t.Errorf("AUTO_MERGE_HUMAN_PATHS: Go %q != Bash %q", got.Scalars["AUTO_MERGE_HUMAN_PATHS"], oracle["AUTO_MERGE_HUMAN_PATHS"])
	}
	if got.Scalars["QUESTIONS_EDITOR"] != oracle["QUESTIONS_EDITOR"] {
		t.Errorf("QUESTIONS_EDITOR: Go %q != Bash %q", got.Scalars["QUESTIONS_EDITOR"], oracle["QUESTIONS_EDITOR"])
	}
}

// TestConfigParityExtraArgsWhitespace verifies whitespace splitting semantics for AGENT_EXTRA_ARGS.
func TestConfigParityExtraArgsWhitespace(t *testing.T) {
	confPath, err := filepath.Abs("testdata/config-extra-args.conf")
	if err != nil {
		t.Fatalf("failed to resolve fixture path: %v", err)
	}

	// Verify that Bash read -ra extra <<< "$EXTRA_ARGS" behaves predictably on multiline/spaced args.
	splitScript := "source \"$1\"\nread -ra extra <<< \"${AGENT_EXTRA_ARGS[split-test]:-}\"\nprintf \"%d\\n\" \"${#extra[@]}\"\nprintf \"%s\\n\" \"${extra[0]}\"\nprintf \"%s\\n\" \"${extra[1]}\"\n"
	cmd := exec.Command("bash", "-c", splitScript, "bash", confPath) //nolint:gosec // G204: fixed test-only script verifying read -ra extra args splitting.
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("failed to inspect Bash extra args splitting: %v", err)
	}

	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	if len(lines) < 3 {
		t.Fatalf("unexpected Bash output: %q", string(out))
	}
	if lines[0] != "3" || lines[1] != "--flag1" || lines[2] != "'quoted" {
		t.Errorf("unexpected read -ra tokens: %v", lines)
	}
}

// TestConfigParityRunPathRawScalar verifies RUN_PATH is preserved as an unsplit raw colon scalar.
func TestConfigParityRunPathRawScalar(t *testing.T) {
	confPath, err := filepath.Abs("testdata/config-extra-args.conf")
	if err != nil {
		t.Fatalf("failed to resolve fixture path: %v", err)
	}

	ctx := ConfigContext{
		Home:       "/tmp/test-parity-home",
		AgentsHome: "/tmp/test-parity-home/agents",
		Hostname:   "parity.example.com",
		UID:        "1234",
		Self:       "/tmp/bin/agent",
	}

	oracle := runBashOracle(t, confPath, ctx)
	if oracle["RUN_PATH"] != "/custom/path/bin:/usr/bin:/bin" {
		t.Errorf("RUN_PATH raw scalar unexpected: %q", oracle["RUN_PATH"])
	}
}

// TestConfigParityStdoutIsolation verifies output noise during sourcing does not break evaluation.
func TestConfigParityStdoutIsolation(t *testing.T) {
	confPath, err := filepath.Abs("testdata/config-noise.conf")
	if err != nil {
		t.Fatalf("failed to resolve fixture path: %v", err)
	}

	ctx := ConfigContext{
		Home:       "/tmp/test-parity-home",
		AgentsHome: "/tmp/test-parity-home/agents",
		Hostname:   "parity.example.com",
		UID:        "1234",
		Self:       "/tmp/bin/agent",
	}

	oracle := runBashOracle(t, confPath, ctx)
	if oracle["INSTANCE_NAME"] != "noisy-instance" {
		t.Errorf("INSTANCE_NAME: unexpected %q", oracle["INSTANCE_NAME"])
	}
}
