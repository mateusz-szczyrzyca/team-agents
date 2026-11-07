package main

import (
	"bytes"
	"context"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// TestStartupConfigPaths retains sourced paths and computed queue settings.
func TestStartupConfigPaths(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("AGENTS_HOME", home)
	writeConfigFixture(t, filepath.Join(home, "agents.conf"), `TASKS=$AGENTS_HOME/custom-tasks
BUS_DIR=$AGENTS_HOME/custom-bus
AGENTBUS_BIN=custom-agentbus
INVESTIGATION_QUEUE_MAX=9
declare -A INVESTIGATORS_TYPE=([first]=codex [second]=claude)
`)
	app := newApp(OSExecutor{}, nil, nil)
	if err := app.loadStartupConfig(context.Background(), []string{"agents"}); err != nil {
		t.Fatal(err)
	}
	if app.config == nil || app.tasksDir != filepath.Join(home, "custom-tasks") || app.busDir != filepath.Join(home, "custom-bus") || app.busBin != "custom-agentbus" || app.agentHome != home {
		t.Fatalf("startup paths were not retained: %+v", app)
	}
	if app.config.Scalars["INVESTIGATION_QUEUE_MAX"] != "9" || app.config.Scalars["INV_SLOTS"] != "2" {
		t.Fatalf("queue settings lost: %+v", app.config.Scalars)
	}
}

// TestAgentsRouteDefaults sends oracle queue capacities for an unconfigured test app.
func TestAgentsRouteDefaults(t *testing.T) {
	ex := &agentsExecutor{t: t}
	app := newApp(ex, nil, nil)
	app.tasksDir = t.TempDir()
	app.busBin = "fixture-agentbus"
	if err := app.routeAgents(context.Background(), io.Discard); err != nil {
		t.Fatal(err)
	}
	args := ex.calls[len(ex.calls)-1].Args
	if !reflect.DeepEqual(args[len(args)-4:], []string{"--queue-max", "3", "--inv-slots", "1"}) {
		t.Fatalf("wrong queue defaults: %v", args)
	}
}

// TestAgentsUnterminatedSummary preserves head's treatment of a missing newline.
func TestAgentsUnterminatedSummary(t *testing.T) {
	ex := &startupSummaryExecutor{}
	app := newApp(ex, nil, nil)
	var output bytes.Buffer
	if err := app.cmdAgents(&output, &Config{Scalars: map[string]string{}}, t.TempDir(), t.TempDir()); err != nil {
		t.Fatal(err)
	}
	if output.String() != "summary" {
		t.Fatalf("summary changed: %q", output.String())
	}
}

// startupSummaryExecutor emits one bus summary without a newline.
type startupSummaryExecutor struct{}

// TestStartupMissingConfig rejects missing production configuration before dispatch.
func TestStartupMissingConfig(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("AGENTS_HOME", home)
	err := newApp(OSExecutor{}, nil, nil).loadStartupConfig(context.Background(), []string{"status"})
	if err == nil || err.Cause != LoadConfigShell {
		t.Fatalf("missing configuration should fail: %v", err)
	}
}

// TestAgentsRouteLoadedConfig forwards configured agent rows and queue capacities.
func TestAgentsRouteLoadedConfig(t *testing.T) {
	ex := &agentsExecutor{t: t}
	app := newApp(ex, nil, nil)
	app.tasksDir = t.TempDir()
	app.config = agentsFixtureConfig()
	var output bytes.Buffer
	if err := app.routeAgents(context.Background(), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), "quoted\"model") {
		t.Fatalf("configured agent rows missing: %q", output.String())
	}
	args := ex.calls[len(ex.calls)-1].Args
	if !reflect.DeepEqual(args[len(args)-4:], []string{"--queue-max", "7", "--inv-slots", "3"}) {
		t.Fatalf("configured queue capacities lost: %v", args)
	}
}

// TestStartupHostnamePrecedence verifies inherited HOSTNAME takes precedence over os.Hostname.
func TestStartupHostnamePrecedence(t *testing.T) {
	t.Setenv("HOSTNAME", "custom-inherited-hostname.example")
	home := t.TempDir()
	confDir := filepath.Join(home, "agents")
	if err := os.MkdirAll(confDir, 0700); err != nil {
		t.Fatal(err)
	}
	writeConfigFixture(t, filepath.Join(confDir, "agents.conf"), "")
	app := newApp(OSExecutor{}, nil, nil)
	t.Setenv("HOME", home)
	t.Setenv("AGENTS_HOME", confDir) // its own configuration, never the one of whoever runs the tests
	if err := app.loadStartupConfig(context.Background(), nil); err != nil {
		t.Fatalf("loadStartupConfig failed: %v", err)
	}
	if got := app.config.Scalars["INSTANCE_NAME"]; got != "custom-inherited-hostname" {
		t.Fatalf("INSTANCE_NAME = %q, want custom-inherited-hostname", got)
	}
}

// Execute provides bus output while preserving any destination failure.
func (*startupSummaryExecutor) Execute(_ context.Context, command Command) (CommandResult, *ExecuteError) {
	if _, err := io.Copy(command.Stdout, strings.NewReader("summary")); err != nil {
		return CommandResult{}, fromProcess(ExecuteWait, processErr(command.Name, err, "write summary"))
	}
	return CommandResult{}, nil
}

// TestStartupConfigExportedEnvPropagated ensures exports in configuration reach subprocesses.
func TestStartupConfigExportedEnvPropagated(t *testing.T) {
	root := t.TempDir()
	binary := filepath.Join(root, "agent-go")
	var buildOut bytes.Buffer
	res, berr := (OSExecutor{}).Execute(context.Background(), Command{Name: "go", Args: []string{"build", "-o", binary, "."}, Stdout: &buildOut, Stderr: &buildOut})
	if berr != nil || res.ExitCode != 0 {
		t.Fatalf("build: %v: %s", berr, buildOut.String())
	}
	home := t.TempDir()
	binDir, confDir, taskDir := filepath.Join(home, "bin"), filepath.Join(home, "agents"), filepath.Join(home, "agents", "tasks", "t1")
	if err := os.MkdirAll(binDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(taskDir, 0o700); err != nil {
		t.Fatal(err)
	}
	stub := "#!/bin/sh\n[ \"$DBUS_SESSION_BUS_ADDRESS\" = \"unix:path=/tmp/test-bus\" ] && exit 0\nexit 1\n"
	if err := os.WriteFile(filepath.Join(binDir, "systemctl"), []byte(stub), 0o700); err != nil { // #nosec G306 -- test executable stub
		t.Fatal(err)
	}
	writeConfigFixture(t, filepath.Join(confDir, "agents.conf"), "export DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/test-bus\n")
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte("ID=t1\nSTATUS=running\nRUN=1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	cmd := Command{Name: binary, Args: []string{"status"}, Env: []string{"HOME=" + home, "AGENTS_HOME=" + confDir, "PATH=" + binDir + ":" + os.Getenv("PATH")}, Stdout: &output, Stderr: &output}
	res, err := (OSExecutor{}).Execute(context.Background(), cmd)
	if err != nil || res.ExitCode != 0 || strings.Contains(output.String(), "running?") || !strings.Contains(output.String(), "running") {
		t.Fatalf("config exports not propagated: %v (exit %d): %s", err, res.ExitCode, output.String())
	}
}
