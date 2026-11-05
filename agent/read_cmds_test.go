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
}

func (m *readCmdsTestExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	m.commands = append(m.commands, cmd)
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
	return CommandResult{ExitCode: 0}, nil
}

func TestCmdBus(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"status": "bus status ok\n",
			"log":    "event1\nevent2\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdBus(context.Background(), "20", &out)
	if err != nil {
		t.Fatalf("cmdBus failed: %v", err)
	}
	got := out.String()
	if !strings.Contains(got, "bus status ok\n\nevent1\nevent2\n") {
		t.Fatalf("cmdBus output unexpected: %q", got)
	}
}

func TestCmdHelpers(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"helper-list": "helper-1 ready\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdHelpers(context.Background(), []string{"--active"}, &out)
	if err != nil {
		t.Fatalf("cmdHelpers failed: %v", err)
	}
	got := out.String()
	if !strings.Contains(got, "helper-1 ready\n") {
		t.Fatalf("cmdHelpers output unexpected: %q", got)
	}
}

func TestCmdInvList(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-list": "1 queue item\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdInv(context.Background(), nil, &out)
	if err != nil {
		t.Fatalf("cmdInv failed: %v", err)
	}
	got := out.String()
	if !strings.Contains(got, "1 queue item\n") || !strings.Contains(got, "All: agent inv --all") {
		t.Fatalf("cmdInv output unexpected: %q", got)
	}
}

func TestCmdInvGet(t *testing.T) {
	tempFile := filepath.Join(t.TempDir(), "inv-record.txt")
	if err := os.WriteFile(tempFile, []byte("Investigation details\n"), 0600); err != nil {
		t.Fatal(err)
	}
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-get": `{"file":"` + tempFile + `"}`,
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdInv(context.Background(), []string{"#42"}, &out)
	if err != nil {
		t.Fatalf("cmdInv get failed: %v", err)
	}
	got := out.String()
	if !strings.Contains(got, "Investigation details\n") {
		t.Fatalf("cmdInv get output unexpected: %q", got)
	}
}

func TestCmdInvListOracleDefaults(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-list": "investigation queue\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdInv(context.Background(), nil, &out)
	if err != nil {
		t.Fatalf("cmdInv failed: %v", err)
	}

	// Verify that oracle defaults are passed: queue-max=3, inv-slots=1
	if len(exec.commands) == 0 {
		t.Fatal("no commands executed")
	}
	cmd := exec.commands[0]
	hasQueueMax3 := false
	hasSlots1 := false
	for i, arg := range cmd.Args {
		if arg == "--queue-max" && i+1 < len(cmd.Args) && cmd.Args[i+1] == "3" {
			hasQueueMax3 = true
		}
		if arg == "--inv-slots" && i+1 < len(cmd.Args) && cmd.Args[i+1] == "1" {
			hasSlots1 = true
		}
	}
	if !hasQueueMax3 {
		t.Fatalf("--queue-max 3 not found in args: %v", cmd.Args)
	}
	if !hasSlots1 {
		t.Fatalf("--inv-slots 1 not found in args: %v", cmd.Args)
	}
}

func TestCmdInvConfiguredCapacities(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-list": "investigation queue\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"
	app.config = &Config{
		Scalars: map[string]string{
			"INVESTIGATION_QUEUE_MAX": "7",
			"INV_SLOTS":               "2",
		},
	}

	var out bytes.Buffer
	err := app.cmdInv(context.Background(), nil, &out)
	if err != nil {
		t.Fatalf("cmdInv failed: %v", err)
	}

	if len(exec.commands) == 0 {
		t.Fatal("no commands executed")
	}
	cmd := exec.commands[0]
	hasQueueMax7 := false
	hasSlots2 := false
	for i, arg := range cmd.Args {
		if arg == "--queue-max" && i+1 < len(cmd.Args) && cmd.Args[i+1] == "7" {
			hasQueueMax7 = true
		}
		if arg == "--inv-slots" && i+1 < len(cmd.Args) && cmd.Args[i+1] == "2" {
			hasSlots2 = true
		}
	}
	if !hasQueueMax7 || !hasSlots2 {
		t.Fatalf("expected --queue-max 7 --inv-slots 2, got: %v", cmd.Args)
	}
}

func TestCmdNotes(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"notes": "notes content\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"
	app.workTree = "/tmp/myrepo"

	var out bytes.Buffer
	err := app.cmdNotes(context.Background(), []string{"--section", "S"}, &out)
	if err != nil {
		t.Fatalf("cmdNotes failed: %v", err)
	}
	got := out.String()
	if !strings.Contains(got, "notes content\n") {
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
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"
	app.workTree = "/home/dev/agents/worktrees/my-awesome-repo/task-456"

	var out bytes.Buffer
	err := app.cmdNotes(context.Background(), []string{"--section", "S"}, &out)
	if err != nil {
		t.Fatalf("cmdNotes failed: %v", err)
	}

	var notesCmd *Command
	for i := range exec.commands {
		if strings.Contains(strings.Join(exec.commands[i].Args, " "), "notes") {
			notesCmd = &exec.commands[i]
		}
	}
	if notesCmd == nil {
		t.Fatal("notes bus command was not executed")
	}
	hasRepo := false
	for i, arg := range notesCmd.Args {
		if arg == "--repo" && i+1 < len(notesCmd.Args) && notesCmd.Args[i+1] == "my-awesome-repo" {
			hasRepo = true
		}
	}
	if !hasRepo {
		t.Fatalf("expected --repo my-awesome-repo in bus command args, got: %v", notesCmd.Args)
	}
}

func TestCmdInvEmptyArgQueue(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-list": "queue content\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdInv(context.Background(), []string{""}, &out)
	if err != nil {
		t.Fatalf("cmdInv failed on empty string arg: %v", err)
	}

	if len(exec.commands) == 0 || exec.commands[0].Args[0] != "inv-list" {
		t.Fatalf("expected inv-list for empty arg, got commands: %v", exec.commands)
	}
}
