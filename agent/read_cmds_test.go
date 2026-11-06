package main

import (
	"bytes"
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type readCmdsTestExecutor struct {
	commands  []Command
	responses map[string]string
	stderrs   map[string]string
	exitCodes map[string]int
}

func (m *readCmdsTestExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	m.commands = append(m.commands, cmd)
	if cmd.Name == "logger" {
		code := 0
		if m.exitCodes != nil {
			code = m.exitCodes["logger"]
		}
		return CommandResult{ExitCode: code}, nil
	}
	key := strings.Join(append([]string{cmd.Name}, cmd.Args...), " ")
	for k, out := range m.responses {
		if strings.Contains(key, k) {
			if cmd.Stdout != nil {
				if _, err := io.WriteString(cmd.Stdout, out); err != nil {
					return CommandResult{}, opErr(ExecuteWait, "write stdout: %s", err.Error())
				}
			}
			return CommandResult{ExitCode: 0}, nil
		}
	}
	for k, errText := range m.stderrs {
		if strings.Contains(key, k) {
			if cmd.Stderr != nil {
				if _, err := io.WriteString(cmd.Stderr, errText); err != nil {
					return CommandResult{}, opErr(ExecuteWait, "write stderr: %s", err.Error())
				}
			}
			exitCode := 1
			if m.exitCodes != nil {
				if code, ok := m.exitCodes[k]; ok {
					exitCode = code
				}
			}
			return CommandResult{ExitCode: exitCode}, nil
		}
	}
	return CommandResult{ExitCode: 0}, nil
}

// newReadCmdsTestApp creates a test app with deterministic bus paths.
func newReadCmdsTestApp(exec Executor) *App {
	app := newApp(exec, nil, nil)
	app.busBin, app.busDir = "/bin/agentbus", "/tmp/bus"
	return app
}

func TestCmdBus(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"status": "bus status ok\n", "log": "event1\nevent2\n"}}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	if err := app.cmdBus(context.Background(), "20", &out); err != nil {
		t.Fatalf("cmdBus failed: %v", err)
	}
	if got := out.String(); !strings.Contains(got, "bus status ok\n\nevent1\nevent2\n") {
		t.Fatalf("cmdBus output unexpected: %q", got)
	}
}

func TestCmdHelpers(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"helper-list": "helper-1 ready\n"}}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	if err := app.cmdHelpers(context.Background(), []string{"--active"}, &out); err != nil {
		t.Fatalf("cmdHelpers failed: %v", err)
	}
	if got := out.String(); !strings.Contains(got, "helper-1 ready\n") {
		t.Fatalf("cmdHelpers output unexpected: %q", got)
	}
}

func TestCmdInvList(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"inv-list": "1 queue item\n"}}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	if err := app.cmdInv(context.Background(), nil, &out); err != nil {
		t.Fatalf("cmdInv failed: %v", err)
	}
	if got := out.String(); !strings.Contains(got, "1 queue item\n") || !strings.Contains(got, "All: agent inv --all") {
		t.Fatalf("cmdInv output unexpected: %q", got)
	}
}

func TestCmdInvGet(t *testing.T) {
	tempFile := filepath.Join(t.TempDir(), "inv-record.txt")
	if err := os.WriteFile(tempFile, []byte("Investigation details\n"), 0600); err != nil {
		t.Fatal(err)
	}
	exec := &readCmdsTestExecutor{responses: map[string]string{"inv-get": `{"file":"` + tempFile + `"}`}}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	if err := app.cmdInv(context.Background(), []string{"#42"}, &out); err != nil {
		t.Fatalf("cmdInv get failed: %v", err)
	}
	if got := out.String(); !strings.Contains(got, "Investigation details\n") {
		t.Fatalf("cmdInv get output unexpected: %q", got)
	}
}

func TestCmdInvListOracleDefaults(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"inv-list": "investigation queue\n"}}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	if err := app.cmdInv(context.Background(), nil, &out); err != nil {
		t.Fatalf("cmdInv failed: %v", err)
	}
	if len(exec.commands) == 0 {
		t.Fatal("no commands executed")
	}
	argsStr := strings.Join(exec.commands[0].Args, " ")
	if !strings.Contains(argsStr, "--queue-max 3") || !strings.Contains(argsStr, "--inv-slots 1") {
		t.Fatalf("defaults missing in args: %v", exec.commands[0].Args)
	}
}

func TestCmdInvConfiguredCapacities(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"inv-list": "investigation queue\n"}}
	app := newReadCmdsTestApp(exec)
	app.config = &Config{Scalars: map[string]string{"INVESTIGATION_QUEUE_MAX": "7", "INV_SLOTS": "2"}}
	var out bytes.Buffer
	if err := app.cmdInv(context.Background(), nil, &out); err != nil {
		t.Fatalf("cmdInv failed: %v", err)
	}
	if len(exec.commands) == 0 {
		t.Fatal("no commands executed")
	}
	argsStr := strings.Join(exec.commands[0].Args, " ")
	if !strings.Contains(argsStr, "--queue-max 7") || !strings.Contains(argsStr, "--inv-slots 2") {
		t.Fatalf("expected --queue-max 7 --inv-slots 2, got: %v", exec.commands[0].Args)
	}
}

func TestCmdNotes(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"notes": "notes content\n"}}
	app := newReadCmdsTestApp(exec)
	app.workTree = "/tmp/myrepo"
	var out bytes.Buffer
	if err := app.cmdNotes(context.Background(), []string{"--section", "S"}, &out); err != nil {
		t.Fatalf("cmdNotes failed: %v", err)
	}
	if got := out.String(); !strings.Contains(got, "notes content\n") {
		t.Fatalf("cmdNotes output unexpected: %q", got)
	}
}

func TestCmdNotesGitRepoDetection(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"show-toplevel":  "/home/dev/agents/worktrees/my-awesome-repo/task-456\n",
			"git-common-dir": "/home/dev/agents/worktrees/my-awesome-repo/.git\n",
			"notes":          "awesome notes\n",
		},
	}
	app := newReadCmdsTestApp(exec)
	app.workTree = "/home/dev/agents/worktrees/my-awesome-repo/task-456"
	var out bytes.Buffer
	if err := app.cmdNotes(context.Background(), []string{"--section", "S"}, &out); err != nil {
		t.Fatalf("cmdNotes failed: %v", err)
	}
	var notesCmd *Command
	for i := range exec.commands {
		if strings.Contains(strings.Join(exec.commands[i].Args, " "), "notes") {
			notesCmd = &exec.commands[i]
		}
	}
	if notesCmd == nil || !strings.Contains(strings.Join(notesCmd.Args, " "), "--repo my-awesome-repo") {
		t.Fatalf("expected --repo my-awesome-repo in bus command args, got: %v", exec.commands)
	}
}

func TestCmdInvEmptyArgQueue(t *testing.T) {
	exec := &readCmdsTestExecutor{responses: map[string]string{"inv-list": "queue content\n"}}
	app := newReadCmdsTestApp(exec)
	var out bytes.Buffer
	if err := app.cmdInv(context.Background(), []string{""}, &out); err != nil {
		t.Fatalf("cmdInv failed on empty string arg: %v", err)
	}
	if len(exec.commands) == 0 || exec.commands[0].Args[0] != "inv-list" {
		t.Fatalf("expected inv-list for empty arg, got commands: %v", exec.commands)
	}
}
