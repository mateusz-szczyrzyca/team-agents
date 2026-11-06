package main

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// TestConfigShellTransport exercises real Bash without touching the installation.
func TestConfigShellTransport(t *testing.T) {
	home := t.TempDir()
	hook := filepath.Join(home, "hook")
	writeConfigFixture(t, hook, `printf ran > "$HOME/hook-ran"`)
	file := filepath.Join(home, "config ; literal")
	writeConfigFixture(t, file, `printf 'ordinary stdout\n'
printf 'ordinary stderr\n' >&2
AGENT_LANGUAGE=$'line1\n"quoted"\t\\'
AGENT_SERENA=
declare -a NORMAL_AGENTS=([0]=first [8]='last member')
declare -a WORKER_AGENTS=()
declare -A AGENT_TYPE=([first]=codex [my-key]=claude [key.sub]=gemini)
declare -A AGENT_MODEL=()
UNRECOGNIZED_SETTING=private
`)
	req := ConfigRequest{File: file, Env: []string{"PATH=/usr/bin:/bin", "BASH_ENV=" + hook, "ENV=" + hook}, Context: ConfigContext{Home: home, Hostname: "node.example", Self: "/bin/candidate"}}
	cfg, err := newApp(OSExecutor{}, nil, nil).loadShellConfig(context.Background(), req)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Scalars["AGENT_LANGUAGE"] != "line1\n\"quoted\"\t\\" {
		t.Fatalf("scalar changed: %q", cfg.Scalars["AGENT_LANGUAGE"])
	}
	if value, ok := cfg.Scalars["AGENT_SERENA"]; !ok || value != "" {
		t.Fatal("explicit empty scalar lost")
	}
	if !reflect.DeepEqual(cfg.Indexed["NORMAL_AGENTS"], map[string]string{"0": "first", "8": "last member"}) {
		t.Fatalf("sparse indices changed: %#v", cfg.Indexed)
	}
	if values, ok := cfg.Indexed["WORKER_AGENTS"]; !ok || len(values) != 0 {
		t.Fatal("declared empty indexed array lost")
	}
	if !reflect.DeepEqual(cfg.Associative["AGENT_TYPE"], map[string]string{"first": "codex", "my-key": "claude", "key.sub": "gemini"}) {
		t.Fatalf("associative keys changed: %#v", cfg.Associative)
	}
	if values, ok := cfg.Associative["AGENT_MODEL"]; !ok || len(values) != 0 {
		t.Fatal("declared empty associative array lost")
	}
	_, unknown := cfg.Scalars["UNRECOGNIZED_SETTING"]
	_, unset := cfg.Scalars["NOTIFY_CMD"]
	if unknown || unset {
		t.Fatal("unknown or unset setting serialized")
	}
	if cfg.Scalars["TASKS"] != home+"/agents/tasks" || cfg.Scalars["SELF"] != req.Context.Self || cfg.Scalars["CONF"] != file {
		t.Fatalf("startup context missing: %#v", cfg.Scalars)
	}
	if _, err := os.Stat(filepath.Join(home, "hook-ran")); !os.IsNotExist(err) {
		t.Fatalf("startup hook was evaluated: %v", err)
	}
}

// TestConfigLoadedDefaults exercises the application boundary after Bash returns a snapshot.
func TestConfigLoadedDefaults(t *testing.T) {
	home := t.TempDir()
	file := filepath.Join(home, "settings")
	writeConfigFixture(t, file, "AGENT_LANGUAGE=Polish")
	cfg, err := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: file, Env: []string{}, Context: ConfigContext{Home: home}})
	if err != nil || cfg.Scalars["AGENT_LANGUAGE"] != "Polish" || cfg.Scalars["RUN_PATH"] != home+"/.local/bin:/usr/local/bin:/usr/bin:/bin" {
		t.Fatalf("loaded defaults differ: %#v, %v", cfg, err)
	}
}

// TestConfigShellEnvironment verifies replacement and inherited environments remain distinct.
func TestConfigShellEnvironment(t *testing.T) {
	t.Setenv("AGENT_CFG_TEST_EXTRA", "inherited")
	home := t.TempDir()
	file := filepath.Join(home, "settings")
	writeConfigFixture(t, file, `AGENT_LANGUAGE=${AGENT_CFG_TEST_EXTRA-absent}`)
	for _, explicit := range []bool{false, true} {
		req := ConfigRequest{File: file, Context: ConfigContext{Home: home}}
		want := "inherited"
		if explicit {
			req.Env = []string{"HOME=/wrong", "PATH=/usr/bin:/bin"}
			want = "absent"
		}
		cfg, err := newApp(OSExecutor{}, nil, nil).loadShellConfig(context.Background(), req)
		if err != nil {
			t.Fatal(err)
		}
		if cfg.Scalars["AGENT_LANGUAGE"] != want || cfg.Scalars["HOME"] != home {
			t.Fatalf("explicit=%v: environment precedence changed", explicit)
		}
	}
}

// TestConfigShellFailures checks exact causes and prevents output values leaking into errors.
func TestConfigShellFailures(t *testing.T) {
	for _, script := range []string{"if [ true", "false\nAGENT_LANGUAGE=unreachable", "exit 7", "printf 'private-marker' >&2; false"} {
		file := filepath.Join(t.TempDir(), "settings")
		writeConfigFixture(t, file, script)
		_, err := newApp(OSExecutor{}, nil, nil).loadShellConfig(context.Background(), ConfigRequest{File: file, Env: []string{}, Context: ConfigContext{Home: t.TempDir()}})
		if err == nil || err.Cause != LoadConfigShell || strings.Contains(err.Error(), "private-marker") {
			t.Fatalf("expected redacted Shell failure, got %v", err)
		}
	}
	for _, app := range []*App{newApp(nil, nil, nil), newApp(OSExecutor{}, nil, nil)} {
		_, err := app.loadShellConfig(context.Background(), ConfigRequest{File: filepath.Join(t.TempDir(), "missing"), Env: []string{}})
		if err == nil || err.Cause != LoadConfigShell {
			t.Fatalf("expected missing/denied Shell failure, got %v", err)
		}
	}
	_, err := newApp(configOutputExecutor{payload: "truncated"}, nil, nil).loadShellConfig(context.Background(), ConfigRequest{})
	if err == nil || err.Cause != LoadConfigProtocol {
		t.Fatalf("expected Protocol failure, got %v", err)
	}
}

// TestConfigShellCompletion rejects a successful shell exit that bypassed serialization.
func TestConfigShellCompletion(t *testing.T) {
	file := filepath.Join(t.TempDir(), "settings")
	writeConfigFixture(t, file, "exit 0")
	_, err := newApp(OSExecutor{}, nil, nil).loadShellConfig(context.Background(), ConfigRequest{File: file, Env: []string{}})
	if err == nil || err.Cause != LoadConfigProtocol {
		t.Fatalf("expected incomplete snapshot failure, got %v", err)
	}
}

// TestConfigNULRecords validates strict shape records independently of shell quoting.
func TestConfigNULRecords(t *testing.T) {
	good := "S\x00AGENT_LANGUAGE\x00\x00\x00I\x00NORMAL_AGENTS\x00\x00\x00i\x00NORMAL_AGENTS\x008\x00member\x00A\x00AGENT_MODEL\x00\x00\x00a\x00AGENT_MODEL\x00key.sub\x00m\x00"
	cfg, err := decodeConfigNUL([]byte(good))
	if err != nil || cfg.Scalars["AGENT_LANGUAGE"] != "" || cfg.Indexed["NORMAL_AGENTS"]["8"] != "member" || cfg.Associative["AGENT_MODEL"]["key.sub"] != "m" {
		t.Fatalf("valid records rejected: %#v, %v", cfg, err)
	}
	for _, bad := range []string{
		"truncated", "S\x00AGENT_LANGUAGE\x00\x00value", "S\x00AGENT_LANGUAGE\x00\x00",
		"X\x00AGENT_LANGUAGE\x00\x00value\x00", "S\x00UNKNOWN\x00\x00value\x00",
		"S\x00AGENT_LANGUAGE\x00key\x00value\x00", "i\x00NORMAL_AGENTS\x000\x00member\x00",
		"I\x00NORMAL_AGENTS\x00\x00\x00i\x00NORMAL_AGENTS\x00bad\x00member\x00",
		"A\x00AGENT_MODEL\x00\x00\x00a\x00AGENT_MODEL\x00\x00m\x00",
		"S\x00AGENT_LANGUAGE\x00\x00a\x00S\x00AGENT_LANGUAGE\x00\x00b\x00",
		"I\x00NORMAL_AGENTS\x00\x00\x00i\x00NORMAL_AGENTS\x000\x00a\x00i\x00NORMAL_AGENTS\x000\x00b\x00",
	} {
		if _, err := decodeConfigNUL([]byte(bad)); err == nil || err.Cause != LoadConfigProtocol {
			t.Fatalf("malformed records accepted: %q, %v", bad, err)
		}
	}
}

// TestConfigNames prevents required maps and context variables disappearing from snapshots.
func TestConfigNames(t *testing.T) {
	names := make(map[string]bool)
	for _, name := range configNames() {
		if names[name] {
			t.Fatalf("duplicate allowlist name: %s", name)
		}
		names[name] = true
	}
	for _, name := range []string{"AGENT_TYPE", "AGENT_EXTRA_ARGS", "RUN_PATH", "HOME", "AGENTS_HOME", "HOSTNAME", "SELF", "CONF", "TASKS", "PROMPTS", "BUS_DIR", "LOG_DIR"} {
		if !names[name] {
			t.Errorf("missing setting: %s", name)
		}
	}
}

// writeConfigFixture writes executable shell text only inside the owning test directory.
func writeConfigFixture(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0600); err != nil {
		t.Fatal(err)
	}
}

// configOutputExecutor supplies protocol bytes without starting a process.
type configOutputExecutor struct {
	payload string // artificial stdout channel for protocol rejection tests
}

// Execute delivers malformed protocol through the same injected stream as OSExecutor.
func (e configOutputExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	if _, err := io.WriteString(cmd.Stdout, e.payload); err != nil {
		panic(err) // The test uses a bytes.Buffer, whose writes cannot fail.
	}
	return CommandResult{}, nil
}

// TestConfigEnvReadFailure ensures non-ENOENT environment read errors fail configuration load.
func TestConfigEnvReadFailure(t *testing.T) {
	conf := filepath.Join(t.TempDir(), "agents.conf")
	writeConfigFixture(t, conf, "trap 'chmod 000 \"$_agent_cfg_env\"' EXIT\n")
	if _, err := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), ConfigRequest{File: conf}); err == nil || err.Cause != LoadConfigShell {
		t.Fatalf("loadConfig = %v, want LoadConfigShell", err)
	}
}
