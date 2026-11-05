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
