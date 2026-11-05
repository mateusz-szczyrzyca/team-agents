package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestCmdStatusHeader verifies the output format header is correct.
func TestCmdStatusHeader(t *testing.T) {
	app := &App{
		Executor: &appExecutor{},
		tasksDir: t.TempDir(),
		Clock:    appClock{},
		Random:   appRandom{},
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	lines := strings.Split(strings.TrimSpace(output), "\n")
	if len(lines) == 0 {
		t.Fatal("no output from cmdStatus")
	}

	header := lines[0]
	expectedFields := []string{"TASK", "AGENT", "STATUS", "ROUND", "RUN", "REPO"}
	for _, field := range expectedFields {
		if !strings.Contains(header, field) {
			t.Errorf("header missing field %q: %s", field, header)
		}
	}
}

// TestCmdStatusListing verifies tasks are listed from $TASKS/*/meta.env.
func TestCmdStatusListing(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create test tasks with metadata
	task1Dir := filepath.Join(tasksDir, "task1")
	if err := os.MkdirAll(task1Dir, 0750); err != nil {
		t.Fatal(err)
	}
	metaFile := filepath.Join(task1Dir, "meta.env")
	meta1 := "ID=task1\nAGENT=agent-1\nSTATUS=done\nROUND=1\nRUN=1\nREPO_NAME=test\n"
	if err := os.WriteFile(metaFile, []byte(meta1), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "task1") {
		t.Errorf("output missing task1: %s", output)
	}
	if !strings.Contains(output, "agent-1") {
		t.Errorf("output missing agent-1: %s", output)
	}
	if !strings.Contains(output, "done") {
		t.Errorf("output missing done status: %s", output)
	}
}

// TestCmdStatusInvestigatorNaming verifies special naming for investigations.
func TestCmdStatusInvestigatorNaming(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create investigation task without INVESTIGATION_TEAM
	invDir := filepath.Join(tasksDir, "inv-task")
	if err := os.MkdirAll(invDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=inv-task\nAGENT=inv-agent\nKIND=investigation\nINV_ID=123\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := os.WriteFile(filepath.Join(invDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "inv#123") {
		t.Errorf("output missing 'inv#123' for investigation: %s", output)
	}
}

// TestCmdStatusMissingFields verifies missing fields are shown as '?'.
func TestCmdStatusMissingFields(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with missing fields
	taskDir := filepath.Join(tasksDir, "partial-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=partial-task\n" // only ID, all other fields missing
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "partial-task") {
		t.Errorf("output missing task ID: %s", output)
	}
	if !strings.Contains(output, "?") {
		t.Errorf("output missing '?' for missing fields: %s", output)
	}
}

// TestCmdStatusSkipsHiddenDirectories verifies that directories starting with . are skipped.
func TestCmdStatusSkipsHiddenDirectories(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create hidden task directory
	hiddenDir := filepath.Join(tasksDir, ".hidden-task")
	if err := os.MkdirAll(hiddenDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=.hidden-task\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(hiddenDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	// Create normal task
	normalDir := filepath.Join(tasksDir, "normal-task")
	if err := os.MkdirAll(normalDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta = "ID=normal-task\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(normalDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	if strings.Contains(output, ".hidden-task") {
		t.Errorf("output should not include hidden task: %s", output)
	}
	if !strings.Contains(output, "normal-task") {
		t.Errorf("output missing normal-task: %s", output)
	}
}

// TestCmdStatusInvestigationEmptyINVID verifies investigation with empty INV_ID shows 'inv#?'.
func TestCmdStatusInvestigationEmptyINVID(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create investigation task with empty INV_ID
	invDir := filepath.Join(tasksDir, "inv-empty")
	if err := os.MkdirAll(invDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=inv-empty\nKIND=investigation\nINV_ID=\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(invDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "inv#?") {
		t.Errorf("output missing 'inv#?' for investigation with empty INV_ID: %s", output)
	}
}

// TestCmdStatusUnitActiveSuffix verifies running/starting status appends ? when unit is inactive.
func TestCmdStatusUnitActiveSuffix(t *testing.T) {
	tasksDir := t.TempDir()

	// Create mock executor that reports unit as inactive (exit code 1)
	exec := &statusTestExecutor{
		unitActive: false, // systemctl is-active returns 1 (inactive)
	}

	app := &App{
		Executor:  exec,
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create running task
	taskDir := filepath.Join(tasksDir, "running-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=running-task\nSTATUS=running\nRUN=1\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdStatus(context.Background(), app, &out)
	if err != nil {
		t.Fatalf("cmdStatus failed: %v", err)
	}

	output := out.String()
	// Should show running? (with ?) when unit is inactive
	if !strings.Contains(output, "running?") {
		t.Errorf("output missing 'running?' suffix for inactive unit: %s", output)
	}
}

type statusTestExecutor struct {
	unitActive bool // false means unit is inactive (exit code 1)
}

func (e *statusTestExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	// Mock systemctl is-active check
	if len(cmd.Args) >= 3 && cmd.Args[1] == "is-active" {
		if e.unitActive {
			return CommandResult{ExitCode: 0}, nil
		}
		return CommandResult{ExitCode: 1}, nil
	}
	return CommandResult{ExitCode: 0}, nil
}
