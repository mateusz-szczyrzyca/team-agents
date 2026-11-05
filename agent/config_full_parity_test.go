package main

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

// runBashNULOracle runs bin/agent's startup section under set -euo pipefail and serializes declared allowlisted settings.
func runBashNULOracle(t *testing.T, confPath string, ctx ConfigContext) Config {
	t.Helper()
	repoRoot, err := filepath.Abs("..")
	if err != nil {
		t.Fatalf("failed to determine repository root: %v", err)
	}
	binAgent := filepath.Join(repoRoot, "bin", "agent")

	script := `
set -euo pipefail
CONF="$1"; HOME="$2"; AGENTS_HOME="$3"; HOSTNAME="$4"; SELF="$5"
TASKS="$AGENTS_HOME/tasks"; PROMPTS="$AGENTS_HOME/prompts"
BUS_DIR="$AGENTS_HOME/bus"; LOG_DIR="$AGENTS_HOME/logs"

eval "$(sed -n '/source "\$CONF"/,/export XDG_RUNTIME_DIR/p' "$6")"

shift 6
for var in "$@"; do
    if declare -p "$var" &>/dev/null; then
        decl=$(declare -p "$var")
        flags=${decl#declare -}
        flags=${flags%% *}
        if [[ $flags == *A* || $flags == *a* ]]; then
            tag=I; elem=i
            if [[ $flags == *A* ]]; then tag=A; elem=a; fi
            printf '%s\0%s\0\0\0' "$tag" "$var"
            declare -n ref="$var"
            for k in "${!ref[@]}"; do
                printf '%s\0%s\0%s\0%s\0' "$elem" "$var" "$k" "${ref[$k]}"
            done
            unset -n ref
        elif [[ -v $var ]]; then
            printf 'S\0%s\0\0%s\0' "$var" "${!var}"
        fi
    fi
done
`
	names := configNames()
	if len(names) == 0 {
		names = append([]string{
			"HOME", "AGENTS_HOME", "HOSTNAME", "SELF", "CONF", "TASKS", "PROMPTS", "BUS_DIR", "LOG_DIR",
			"AGENT_TYPE", "AGENT_EXTRA_ARGS", "AGENT_MODEL", "NORMAL_AGENTS", "WORKER_AGENTS", "INSTANCE_CAPABILITIES",
			"INVESTIGATORS_TYPE", "INVESTIGATORS_MODEL", "INVESTIGATORS_EXTRA_ARGS", "INV_SLOTS", "ROUND_MAX_LINES_REPO", "NODE_REPOS",
		}, oracleSettingsList()...)
	}
	args := append([]string{"-c", script, "bash", confPath, ctx.Home, ctx.AgentsHome, ctx.Hostname, ctx.Self, binAgent}, names...)
	cmd := exec.Command("bash", args...) //nolint:gosec // G204: test-only oracle script
	cmd.Env = []string{"PATH=/usr/bin:/bin", "HOME=" + ctx.Home, "UID=" + ctx.UID}
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("bash oracle failed on %s: %v", confPath, err)
	}

	cfg := newConfig()
	fields := bytes.Split(out, []byte{0})
	for i := 0; i+3 < len(fields)-1; i += 4 {
		tag, name, key, val := string(fields[i]), string(fields[i+1]), string(fields[i+2]), string(fields[i+3])
		switch tag {
		case "S":
			cfg.Scalars[name] = val
		case "I":
			cfg.Indexed[name] = make(map[string]string)
		case "A":
			cfg.Associative[name] = make(map[string]string)
		case "i":
			if cfg.Indexed[name] == nil {
				cfg.Indexed[name] = make(map[string]string)
			}
			cfg.Indexed[name][key] = val
		case "a":
			if cfg.Associative[name] == nil {
				cfg.Associative[name] = make(map[string]string)
			}
			cfg.Associative[name][key] = val
		}
	}
	return cfg
}

// oracleSettingsList lists all documented configuration settings evaluated by the startup oracle.
func oracleSettingsList() []string {
	return strings.Fields("CLAUDE_BIN CODEX_BIN AGENTBUS_BIN RUN_PATH NOTIFY_CMD QUESTIONS_EDITOR VSCODE_SERVER_DIRS AGENT_CONSULT_MIN NOTIFY_DIR NOTIFY_SCRIPT NOTIFY_MAX_CHARS AGENT_LANGUAGE REPO_LANGUAGE GIT_NAME_FORMAT GIT_EMAIL GIT_SIGNING_KEY ENV_NOTES_FILE BRANCH_PREFIX CLAUDE_MAX_TURNS CLAUDE_TOOL_SEARCH CLAUDE_BASH_TIMEOUT_MIN AGY_BIN OPENCODE_BIN OPENCODE_BASH_TIMEOUT_MS AGY_PRINT_TIMEOUT INVESTIGATOR_NAME INVESTIGATOR_TYPE INVESTIGATOR_MODEL INVESTIGATOR_EXTRA_ARGS INVESTIGATION_QUEUE_MAX AGENT_INVESTIGATION_SEED AGENT_HANG_MIN AGENT_HANG_STRIKES AGENT_INVESTIGATION_EXCLUSIVE AGENT_IDLE_RUN_SEC AGENT_WAKE_LOOP_MAX AGENT_WAKE_RATE_MAX AGENT_WAKE_LOOP_PAUSE_MIN AGENT_QUOTA_PAUSE_MIN AGENT_RELEASE_PAUSE_MIN AGENT_SUPERVISE_GRACE_SEC AGENT_RESTART_MAX AGENT_RESTART_GRACE_MIN AGENT_TEAM_ABANDON_MIN AGENT_RATE_LIMIT_PAUSE_MIN AGENT_NETWORK_PAUSE_MIN AGENT_NETWORK_MAX_PAUSES AGENT_GITHUB_COLORS AGENT_REVIEW_DRAW AGENT_REVIEW_ROTATE_WAIT_MIN REVIEW_HISTORY_MAX AGENT_SERENA AGENT_PROMPT_CHECKPOINTS AGENT_FRESH_SESSION_TOKENS AGENT_FRESH_SESSION_RATIO AGENT_LONG_RUN_STEPS AGENT_LONG_RUN_HARD AGENT_PROMPT_ARG_MAX AGENT_LIMIT_TAIL_LINES AGENTLIVE_BIN LIVE_REMIND_MIN AGENT_WEB_LISTEN AGENT_WEB_USERS AGENT_WEB_DEVICES AGENT_WEB_URL LEAD_LISTEN LEAD_NODES INSTANCE_NAME NODE_TTL_SEC ROUND_SOFT_LINES ROUND_ALWAYS_EXCLUDE VERIFY_MAX_FIX_PASSES AGENT_REVIEW_GATE AGENT_REVIEW_GATE_MIN AGENT_STAGE_POLL_SEC AGENT_PRECHECK_MAX AUTO_MERGE_HUMAN_PATHS AUTO_MERGE_HOLD_TEST_CHANGES AGENT_REVIEW_WAIT_MIN AGENT_HUMAN_GITHUB ROUND_MAX_LINES ROUND_EXCLUDE ROUND_BOUNCES LEAD_URL NODE_POLL_SEC AUTH_MODEL_ANTHROPIC AUTH_MODEL_OPENAI AUTH_MODEL_GEMINI AUTH_MODEL_OPENROUTER AUTH_MODEL_DEEPINFRA AUTH_OPENROUTER_URL AUTH_DEEPINFRA_URL XDG_RUNTIME_DIR")
}

// TestConfigFullParityFixtures verifies loadConfig output against the Bash NUL oracle for all fixtures.
func TestConfigFullParityFixtures(t *testing.T) {
	home := t.TempDir()
	ctx := ConfigContext{
		Home:       home,
		AgentsHome: home + "/agents",
		Hostname:   "full-parity.example.com",
		UID:        strconv.Itoa(os.Getuid()),
		Self:       home + "/bin/agent",
	}
	cleanEnv := []string{"PATH=/usr/bin:/bin", "HOME=" + home, "UID=" + ctx.UID}
	app := newApp(OSExecutor{}, nil, nil)

	fixtures := []string{os.DevNull, "testdata/config-declared.conf", "testdata/config-legacy-investigator.conf", "testdata/config-empty-unset.conf", "testdata/config-noise.conf"}
	for _, fixture := range fixtures {
		confPath := fixture
		if fixture != os.DevNull {
			p, err := filepath.Abs(fixture)
			if err != nil {
				t.Fatalf("failed to resolve fixture %s: %v", fixture, err)
			}
			confPath = p
		}
		oracle := runBashNULOracle(t, confPath, ctx)
		got, err := app.loadConfig(context.Background(), ConfigRequest{File: confPath, Env: cleanEnv, Context: ctx})
		if err != nil {
			t.Fatalf("%s: loadConfig failed: %v", fixture, err)
		}
		for name, want := range oracle.Scalars {
			if gotVal, ok := got.Scalars[name]; !ok || gotVal != want {
				t.Errorf("%s: scalar %s mismatch: got %q (ok=%t), want %q", fixture, name, gotVal, ok, want)
			}
		}
		for name, want := range oracle.Indexed {
			if !reflect.DeepEqual(got.Indexed[name], want) {
				t.Errorf("%s: indexed %s mismatch: got %#v, want %#v", fixture, name, got.Indexed[name], want)
			}
		}
		for name, want := range oracle.Associative {
			if !reflect.DeepEqual(got.Associative[name], want) {
				t.Errorf("%s: associative %s mismatch: got %#v, want %#v", fixture, name, got.Associative[name], want)
			}
		}
	}
}

// TestConfigContextLazyOverrides exercises exact success and LoadConfigShell failures on lazy context settings.
func TestConfigContextLazyOverrides(t *testing.T) {
	ctx := ConfigContext{Home: "/tmp/h", AgentsHome: "/tmp/h/agents", Hostname: "node.example", UID: "1234"}
	cleanEnv := []string{"PATH=/usr/bin:/bin"}
	app := newApp(OSExecutor{}, nil, nil)

	for _, script := range []string{"unset HOME\n", "unset HOSTNAME\n", "unset AGENTS_HOME\n"} {
		f := filepath.Join(t.TempDir(), "fail")
		writeContextFixture(t, f, script)
		if _, err := app.loadConfig(context.Background(), ConfigRequest{File: f, Env: cleanEnv, Context: ctx}); err == nil || err.Cause != LoadConfigShell {
			t.Errorf("%q: expected LoadConfigShell, got %v", script, err)
		}
	}

	successCases := []struct {
		script, key, want string
	}{
		{"unset HOME\nCLAUDE_BIN=/c/claude\nCODEX_BIN=/c/codex\nAGENTBUS_BIN=/c/bus\nRUN_PATH=/c/bin\nVSCODE_SERVER_DIRS=/c/vs\nAGY_BIN=/c/agy\nOPENCODE_BIN=/c/op\nAGENTLIVE_BIN=/c/live\n", "CLAUDE_BIN", "/c/claude"},
		{"unset HOSTNAME\nINSTANCE_NAME=custom-node\n", "INSTANCE_NAME", "custom-node"},
		{"unset AGENTS_HOME\nNOTIFY_DIR=/c/notify\nENV_NOTES_FILE=/c/notes\n", "NOTIFY_DIR", "/c/notify"},
	}
	for _, tc := range successCases {
		f := filepath.Join(t.TempDir(), "ok")
		writeContextFixture(t, f, tc.script)
		if cfg, err := app.loadConfig(context.Background(), ConfigRequest{File: f, Env: cleanEnv, Context: ctx}); err != nil || cfg.Scalars[tc.key] != tc.want {
			t.Errorf("%q: expected %s=%q, got cfg=%#v, err=%v", tc.script, tc.key, tc.want, cfg, err)
		}
	}
}
