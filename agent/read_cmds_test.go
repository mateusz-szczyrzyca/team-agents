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
	commands   []Command
	responses  map[string]string
	exitCodes  map[string]int // maps command patterns to exit codes
}

func (m *readCmdsTestExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	m.commands = append(m.commands, cmd)
	key := strings.Join(append([]string{cmd.Name}, cmd.Args...), " ")

	// Check for explicit exit code mapping
	exitCode := 0
	for pattern, code := range m.exitCodes {
		if strings.Contains(key, pattern) {
			exitCode = code
			break
		}
	}

	// Check for response output
	for k, out := range m.responses {
		if strings.Contains(key, k) {
			if cmd.Stdout != nil {
				if _, err := io.WriteString(cmd.Stdout, out); err != nil {
					return CommandResult{}, opErr(ExecuteWait, "write stdout: %s", err.Error())
				}
			}
			if exitCode != 0 {
				// Return non-zero exit code even though output was written
				return CommandResult{ExitCode: exitCode}, nil
			}
			return CommandResult{ExitCode: 0}, nil
		}
	}
	return CommandResult{ExitCode: exitCode}, nil
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

// TestCmdBusStatusPreservedOnLogFailure verifies that successful status output is preserved even when log retrieval fails.
func TestCmdBusStatusPreservedOnLogFailure(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"status": "bus status ok\n",
			"log":    "event1\nevent2\n",
		},
		exitCodes: map[string]int{
			"log": 5, // Make log command fail with exit code 5
		},
	}
	app := newApp(exec, nil, nil)
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	var out bytes.Buffer
	err := app.cmdBus(context.Background(), "20", &out)
	if err == nil {
		t.Fatal("cmdBus should fail when log command fails")
	}
	got := out.String()
	// The status output and separator should be preserved in the output even though log failed
	if !strings.Contains(got, "bus status ok\n\n") {
		t.Fatalf("status output not preserved on log failure: %q", got)
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
