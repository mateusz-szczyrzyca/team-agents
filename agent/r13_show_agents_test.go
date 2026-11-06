package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestCmdShowCRLFExactOutput verifies that CRLF endings in task files are formatted properly and verified with exact assertions.
func TestCmdShowCRLFExactOutput(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	taskDir := filepath.Join(tasksDir, "task-crlf")
	if err := os.MkdirAll(taskDir, 0o750); err != nil {
		t.Fatal(err)
	}
	writeTestScript(t, filepath.Join(taskDir, "meta.env"), "ID=task-crlf\r\nSTATUS=done\r\nAGENT=solo\r\n", 0o644)
	writeTestScript(t, filepath.Join(taskDir, "task.md"), "Task instruction line 1\r\nTask instruction line 2\r\n", 0o644)
	writeTestScript(t, filepath.Join(taskDir, "final.md"), "Agent final message\r\n", 0o644)

	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "task-crlf", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	got := out.String()
	if !strings.Contains(got, "\r") {
		t.Fatalf("expected output to contain carriage returns, got: %q", got)
	}

	want := "== task-crlf\n  ID=task-crlf\r\n  STATUS=done\r\n  AGENT=solo\r\n\n== task\nTask instruction line 1\r\nTask instruction line 2\r\n\n== the agent's last message\nAgent final message\r\n"
	if got != want {
		t.Fatalf("unexpected output:\ngot:  %q\nwant: %q", got, want)
	}
}

// TestCmdAgentsSystemctlFailureTreatsAgentAsFree verifies that when systemctl fails to launch, the agent is reported as free.
func TestCmdAgentsSystemctlFailureTreatsAgentAsFree(t *testing.T) {
	tasksDir := t.TempDir()
	taskDir := filepath.Join(tasksDir, "task-sys-fail")
	if err := os.MkdirAll(taskDir, 0o750); err != nil {
		t.Fatal(err)
	}
	writeTestScript(t, filepath.Join(taskDir, "meta.env"), "ID=task-sys-fail\nSTATUS=running\nAGENT=solo\nRUN=1\n", 0o644)

	app := &App{
		tasksDir: tasksDir,
		Executor: executorFunc(func(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "systemctl" {
				return CommandResult{}, opErr(ExecuteStart, "cannot execute systemctl")
			}
			return CommandResult{ExitCode: 0}, nil
		}),
	}

	cfg := &Config{
		Associative: map[string]map[string]string{
			"AGENT_TYPE":  {"solo": "gemini"},
			"AGENT_MODEL": {"solo": "default"},
		},
		Scalars: map[string]string{
			"AGENTBUS_BIN": "agentbus",
		},
	}

	var out bytes.Buffer
	if err := app.cmdAgents(&out, cfg, tasksDir, t.TempDir()); err != nil {
		t.Fatalf("cmdAgents failed: %v", err)
	}

	got := out.String()
	if !strings.Contains(got, "solo") || !strings.Contains(got, "free") {
		t.Fatalf("expected solo agent to be reported as free, got: %q", got)
	}
	if strings.Contains(got, "busy") || strings.Contains(got, "task-sys-fail") {
		t.Fatalf("expected agent not to be reported as busy, got: %q", got)
	}
}
