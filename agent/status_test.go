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
