package main

import (
	"bytes"
	"context"
	"io/ioutil"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestCmdShowTaskNotFound verifies ShowNotFound error is returned.
func TestCmdShowTaskNotFound(t *testing.T) {
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  t.TempDir(),
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "nonexistent-task", &out)
	if err == nil {
		t.Fatal("cmdShow should return error for nonexistent task")
	}
	// Should be a ShowNotFound error
	if err.Cause != ShowNotFound {
		t.Errorf("got error %v, want ShowNotFound", err)
	}
}

// TestCmdShowMetaenv verifies meta.env is printed indented by 2 spaces.
func TestCmdShowMetaenv(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with metadata
	taskDir := filepath.Join(tasksDir, "show-test")
	if err := os.MkdirAll(taskDir, 0755); err != nil {
		t.Fatal(err)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	meta := "ID=show-test\nAGENT=test-agent\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := ioutil.WriteFile(metaFile, []byte(meta), 0644); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "show-test", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "== show-test") {
		t.Errorf("output missing task header: %s", output)
	}
	if !strings.Contains(output, "  ID=show-test") {
		t.Errorf("output missing indented meta.env: %s", output)
	}
}

// TestCmdShowTaskmd verifies task.md is printed under '== task' header.
func TestCmdShowTaskmd(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with task.md
	taskDir := filepath.Join(tasksDir, "taskmd-test")
	if err := os.MkdirAll(taskDir, 0755); err != nil {
		t.Fatal(err)
	}
	meta := "ID=taskmd-test\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := ioutil.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0644); err != nil {
		t.Fatal(err)
	}
	task := "# Test Task\n\nThis is a test task description."
	if err := ioutil.WriteFile(filepath.Join(taskDir, "task.md"), []byte(task), 0644); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "taskmd-test", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "== task") {
		t.Errorf("output missing task.md header: %s", output)
	}
	if !strings.Contains(output, "# Test Task") {
		t.Errorf("output missing task.md content: %s", output)
	}
}

// TestCmdShowQuestions verifies questions file is shown when ROUND > 0.
func TestCmdShowQuestions(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with ROUND > 0 and questions file
	taskDir := filepath.Join(tasksDir, "questions-test")
	if err := os.MkdirAll(taskDir, 0755); err != nil {
		t.Fatal(err)
	}
	meta := "ID=questions-test\nSTATUS=done\nROUND=2\nRUN=1\n"
	if err := ioutil.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0644); err != nil {
		t.Fatal(err)
	}
	questions := "# Questions for Round 2"
	if err := ioutil.WriteFile(filepath.Join(taskDir, "questions-2.md"), []byte(questions), 0644); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "questions-test", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "questions (round 2)") {
		t.Errorf("output missing questions header: %s", output)
	}
}

// TestCmdShowFinalMessage verifies final message from final.md is shown.
func TestCmdShowFinalMessage(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with final.md
	taskDir := filepath.Join(tasksDir, "final-test")
	if err := os.MkdirAll(taskDir, 0755); err != nil {
		t.Fatal(err)
	}
	meta := "ID=final-test\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := ioutil.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0644); err != nil {
		t.Fatal(err)
	}
	final := "Final message from the agent"
	if err := ioutil.WriteFile(filepath.Join(taskDir, "final.md"), []byte(final), 0644); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "final-test", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "== the agent's last message") {
		t.Errorf("output missing final message header: %s", output)
	}
	if !strings.Contains(output, final) {
		t.Errorf("output missing final message content: %s", output)
	}
}

// TestCmdShowReport verifies report is shown from .agents/reports/$id.md.
func TestCmdShowReport(t *testing.T) {
	tasksDir := t.TempDir()
	workTree := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		workTree:  workTree,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task
	taskDir := filepath.Join(tasksDir, "report-test")
	if err := os.MkdirAll(taskDir, 0755); err != nil {
		t.Fatal(err)
	}
	meta := "ID=report-test\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := ioutil.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0644); err != nil {
		t.Fatal(err)
	}

	// Create report file in .agents/reports/
	reportDir := filepath.Join(workTree, ".agents", "reports")
	if err := os.MkdirAll(reportDir, 0755); err != nil {
		t.Fatal(err)
	}
	report := "# Task Report\n\nCompleted successfully."
	if err := ioutil.WriteFile(filepath.Join(reportDir, "report-test.md"), []byte(report), 0644); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "report-test", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "== report:") {
		t.Errorf("output missing report header: %s", output)
	}
}
