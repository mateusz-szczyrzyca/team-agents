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

// TestProductionListings loads real configuration through the compiled entry point.
func TestProductionListings(t *testing.T) {
	root := t.TempDir()
	binary := filepath.Join(root, "agent-go")
	var buildOutput bytes.Buffer
	result, buildErr := (OSExecutor{}).Execute(context.Background(), Command{Name: "go", Args: []string{"build", "-o", binary, "."}, Stdout: &buildOutput, Stderr: &buildOutput})
	if buildErr != nil || result.ExitCode != 0 {
		t.Fatalf("build: %v: %s", buildErr, buildOutput.String())
	}
	for _, customHome := range []bool{false, true} {
		t.Run(map[bool]string{false: "HOME", true: "AGENTS_HOME"}[customHome], func(t *testing.T) {
			home := t.TempDir()
			agentsHome := filepath.Join(home, "agents")
			if customHome {
				agentsHome = filepath.Join(home, "custom")
			}
			if err := os.MkdirAll(agentsHome, 0o700); err != nil {
				t.Fatal(err)
			}
			writeConfigFixture(t, filepath.Join(agentsHome, "agents.conf"), `declare -A AGENT_TYPE=([fixture]=codex)
declare -A AGENT_MODEL=([fixture]=example-model)
declare -A INVESTIGATORS_TYPE=([reviewer]=claude)
declare -A INVESTIGATORS_MODEL=([reviewer]=review-model)
`)
			var output bytes.Buffer
			command := Command{Name: binary, Args: []string{"_agents-json"}, Env: []string{"HOME=" + home, "PATH=" + os.Getenv("PATH")}, Stdout: &output, Stderr: &output}
			if customHome {
				command.Env = append(command.Env, "AGENTS_HOME="+agentsHome)
			}
			result, err := (OSExecutor{}).Execute(context.Background(), command)
			if err != nil || result.ExitCode != 0 || !strings.Contains(output.String(), `"name":"fixture"`) || !strings.Contains(output.String(), `"name":"reviewer"`) {
				t.Fatalf("configured listings missing: %v: %s", err, output.String())
			}
		})
	}
}

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
