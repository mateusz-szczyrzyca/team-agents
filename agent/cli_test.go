package main

import (
	"bytes"
	"context"
	"errors"
	"strings"
	"testing"
)

func TestEmptyArgvRunsHelpWithExitZero(t *testing.T) {
	var stdout, stderr bytes.Buffer
	code := run(nil, &stdout, &stderr)
	if code != 0 {
		t.Fatalf("run(nil) exit code = %d, want 0", code)
	}
	if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
		t.Fatalf("run(nil) stdout does not contain usage: %q", stdout.String())
	}
}

func TestHelpAliasesRunHelpWithExitZero(t *testing.T) {
	aliases := []string{"help", "-h", "--help"}
	for _, alias := range aliases {
		var stdout, stderr bytes.Buffer
		code := run([]string{alias}, &stdout, &stderr)
		if code != 0 {
			t.Fatalf("run([%q]) exit code = %d, want 0", alias, code)
		}
		if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
			t.Fatalf("run([%q]) stdout does not contain usage: %q", alias, stdout.String())
		}
	}
}

func TestUnknownCommandExitsOneWithUsage(t *testing.T) {
	var stdout, stderr bytes.Buffer
	code := run([]string{"unknown-cmd"}, &stdout, &stderr)
	if code != 1 {
		t.Fatalf("run([\"unknown-cmd\"]) exit code = %d, want 1", code)
	}
	if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
		t.Fatalf("run([\"unknown-cmd\"]) stdout does not contain usage: %q", stdout.String())
	}
}

func TestDispatchUnknownCommand(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("unknown-cmd", nil, &stdout, &stderr)
	if err == nil {
		t.Fatal("dispatch(\"unknown-cmd\") returned nil error, want CommandUnknownCommand")
	}
	if err.Cause != CommandUnknownCommand {
		t.Fatalf("dispatch(\"unknown-cmd\") cause = %v, want CommandUnknownCommand", err.Cause)
	}
	if !errors.Is(err, ErrInvalidArgument) {
		t.Fatalf("dispatch(\"unknown-cmd\") error does not unwrap to ErrInvalidArgument")
	}
}

func TestDispatchRecognizesR09Commands(t *testing.T) {
	commands := []string{"status", "ls", "show", "agents", "_agents-json", "bus", "inv", "notes", "helpers"}
	for _, cmd := range commands {
		var stdout, stderr bytes.Buffer
		err := dispatch(cmd, nil, &stdout, &stderr)
		if err != nil && err.Cause == CommandUnknownCommand {
			t.Fatalf("dispatch(%q) returned CommandUnknownCommand, want recognized command", cmd)
		}
	}
}

func TestDispatchShowMissingArg(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("show", nil, &stdout, &stderr)
	if err == nil || err.Cause != CommandInvalidArgument {
		t.Fatalf("dispatch(show, nil) err = %v, want CommandInvalidArgument", err)
	}
}

func TestDispatchShowNotFound(t *testing.T) {
	tempTasks := t.TempDir()
	app := newApp(nil, nil, nil)
	app.tasksDir = tempTasks
	var stdout, stderr bytes.Buffer
	err := app.dispatch(context.Background(), "show", []string{"missing-task"}, &stdout, &stderr)
	if err == nil || err.Cause != CommandNotFound {
		t.Fatalf("dispatch(show, missing-task) err = %v, want CommandNotFound", err)
	}
	if err.Class() != ClassNotFound {
		t.Fatalf("dispatch(show, missing-task) class = %v, want ClassNotFound", err.Class())
	}
}

// TestDispatchWithArgs verifies that dispatching a command with arguments resolves and validates targets.
func TestDispatchWithArgs(t *testing.T) {
	tempTasks := t.TempDir()
	app := newApp(nil, nil, nil)
	app.tasksDir = tempTasks
	var stdout, stderr bytes.Buffer
	err := app.dispatch(context.Background(), "show", []string{"nonexistent-task"}, &stdout, &stderr)
	if err == nil || err.Cause != CommandNotFound {
		t.Fatalf("dispatch(show, [nonexistent-task]) err = %v, want CommandNotFound", err)
	}
}

// TestDelegatedBusExitCodesAndStderr verifies that delegated agentbus exit codes and stderr are preserved without duplication.
func TestDelegatedBusExitCodesAndStderr(t *testing.T) {
	cases := []struct {
		cmd    []string
		code   int
		stderr string
	}{
		{[]string{"inv", "99"}, BusExitNotFound, "agentbus: inv-get: no investigation #99\n"},
		{[]string{"helpers"}, BusExitPermission, "agentbus: permission denied\n"},
		{[]string{"bus", "5"}, BusExitNotFound, "agentbus: bus state not found\n"},
		{[]string{"notes", "myrepo"}, BusExitNotInTeam, "agentbus: not in team\n"},
	}
	for _, tc := range cases {
		t.Run(tc.cmd[0], func(t *testing.T) {
			exec := &busTestExecutor{result: CommandResult{ExitCode: tc.code}, stderr: tc.stderr}
			app := newApp(exec, nil, nil)
			var stdout, stderr bytes.Buffer
			if code := app.run(tc.cmd, &stdout, &stderr); code != tc.code {
				t.Fatalf("run(%v) exit code = %d, want %d", tc.cmd, code, tc.code)
			}
			if tc.stderr != "" && stderr.String() != tc.stderr {
				t.Fatalf("run(%v) stderr = %q, want %q", tc.cmd, stderr.String(), tc.stderr)
			}
		})
	}

	t.Run("agents exits BusExitPermission on bus failure", func(t *testing.T) {
		exec := &busTestExecutor{
			result: CommandResult{ExitCode: BusExitPermission},
			stderr: "agentbus: permission denied\n",
		}
		cfg := &Config{Scalars: map[string]string{"INVESTIGATION_QUEUE_MAX": "3", "INV_SLOTS": "1"}}
		app := newApp(exec, nil, nil)
		app.config = cfg
		app.tasksDir = t.TempDir()
		var stdout, stderr bytes.Buffer
		if code := app.run([]string{"agents"}, &stdout, &stderr); code != BusExitPermission {
			t.Fatalf("run(agents) exit code = %d, want %d", code, BusExitPermission)
		}
	})
}

func TestRunDiagnosticsOnStderr(t *testing.T) {
	app := newApp(nil, nil, nil)
	var stdout, stderr bytes.Buffer
	code := app.run([]string{"show"}, &stdout, &stderr)
	if code != 1 {
		t.Fatalf("run(show) exit code = %d, want 1", code)
	}
	if !strings.Contains(stderr.String(), "agent: usage: agent show <id>\n") {
		t.Fatalf("run(show) stderr = %q, want 'agent: usage: agent show <id>'", stderr.String())
	}

	// Unknown command must not print agent: <message> to stderr (only usage to stdout)
	stdout.Reset()
	stderr.Reset()
	code = app.run([]string{"unknown-cmd"}, &stdout, &stderr)
	if code != 1 {
		t.Fatalf("run(unknown-cmd) exit code = %d, want 1", code)
	}
	if stderr.Len() != 0 {
		t.Fatalf("run(unknown-cmd) stderr = %q, want empty", stderr.String())
	}
}

func TestDispatchAgentsJSON(t *testing.T) {
	var stdout, stderr bytes.Buffer
	err := dispatch("_agents-json", nil, &stdout, &stderr)
	if err != nil {
		t.Fatalf("dispatch(_agents-json, nil) err = %v, want nil", err)
	}
}

func TestDispatchHelp(t *testing.T) {
	for _, cmd := range []string{"help", "-h", "--help"} {
		var stdout, stderr bytes.Buffer
		err := dispatch(cmd, nil, &stdout, &stderr)
		if err != nil {
			t.Fatalf("dispatch(%q) returned error = %v, want nil", cmd, err)
		}
		if !strings.Contains(stdout.String(), "agent run <agent> \"task\"") {
			t.Fatalf("dispatch(%q) stdout does not contain usage", cmd)
		}
	}
}

func TestDispatchShowEmptyArg(t *testing.T) {
	var stdout, stderr bytes.Buffer
	if err := dispatch("show", []string{""}, &stdout, &stderr); err == nil || err.Cause != CommandInvalidArgument {
		t.Fatalf("dispatch(show, empty argument) = %v, want CommandInvalidArgument", err)
	}
}
