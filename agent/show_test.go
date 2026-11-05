package main

import (
	"bytes"
	"context"
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
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	metaFile := filepath.Join(taskDir, "meta.env")
	meta := "ID=show-test\nAGENT=test-agent\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := os.WriteFile(metaFile, []byte(meta), 0600); err != nil {
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
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=taskmd-test\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	task := "# Test Task\n\nThis is a test task description."
	if err := os.WriteFile(filepath.Join(taskDir, "task.md"), []byte(task), 0600); err != nil {
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
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=questions-test\nSTATUS=done\nROUND=2\nRUN=1\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	questions := "# Questions for Round 2"
	if err := os.WriteFile(filepath.Join(taskDir, "questions-2.md"), []byte(questions), 0600); err != nil {
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
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=final-test\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	final := "Final message from the agent"
	if err := os.WriteFile(filepath.Join(taskDir, "final.md"), []byte(final), 0600); err != nil {
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

// TestCmdShowWorkerNoInvestigations verifies worker tasks don't print investigations header when none exist.
func TestCmdShowWorkerNoInvestigations(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create worker task
	taskDir := filepath.Join(tasksDir, "worker-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=worker-task\nKIND=worker\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "worker-task", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	// Should not have investigations header when KIND=worker
	if strings.Contains(output, "== investigations") {
		t.Errorf("worker task should not show empty investigations header: %s", output)
	}
}

// TestCmdShowWorktreeFromMeta verifies WORKTREE is read from meta.env, not app.workTree.
func TestCmdShowWorktreeFromMeta(t *testing.T) {
	tasksDir := t.TempDir()
	appWorkTree := "/wrong/path"
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		workTree:  appWorkTree, // This should NOT be used
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with WORKTREE in meta.env
	taskDir := filepath.Join(tasksDir, "wt-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=wt-task\nWORKTREE=/correct/path\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	// Note: we can't actually create reports at /correct/path, so this test verifies
	// the code attempts to look in the right place

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "wt-task", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	// Verify output doesn't use app.workTree in report path
	if strings.Contains(output, appWorkTree) && strings.Contains(output, "== report:") {
		t.Errorf("report path should use meta.env WORKTREE, not app.workTree: %s", output)
	}
}

// TestCmdShowQuestionsContent verifies questions file content is printed, not just header.
func TestCmdShowQuestionsContent(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with questions
	taskDir := filepath.Join(tasksDir, "q-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=q-task\nROUND=1\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	questions := "# Questions - Round 1\n\n## 1. First question\n**Recommended:** a) Option A"
	if err := os.WriteFile(filepath.Join(taskDir, "questions-1.md"), []byte(questions), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "q-task", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	// Should have questions header
	if !strings.Contains(output, "== questions") {
		t.Errorf("output missing questions header: %s", output)
	}
	// Should have questions CONTENT, not just header
	if !strings.Contains(output, "First question") {
		t.Errorf("output missing questions content: %s", output)
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
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=report-test\nSTATUS=done\nROUND=1\nRUN=1\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	// Create report file in .agents/reports/
	reportDir := filepath.Join(workTree, ".agents", "reports")
	if err := os.MkdirAll(reportDir, 0750); err != nil {
		t.Fatal(err)
	}
	report := "# Task Report\n\nCompleted successfully."
	if err := os.WriteFile(filepath.Join(reportDir, "report-test.md"), []byte(report), 0600); err != nil {
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

// TestCmdShowQuestionsHeaderWithoutFile verifies questions header prints when ROUND > 0 even if file does not exist.
func TestCmdShowQuestionsHeaderWithoutFile(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	taskDir := filepath.Join(tasksDir, "q-missing")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=q-missing\nROUND=2\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "q-missing", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "== questions (round 2):") {
		t.Errorf("output missing questions header: %s", output)
	}
}

// TestCmdShowWorkerWithInvestigations verifies worker task prints investigations section when non-empty.
func TestCmdShowWorkerWithInvestigations(t *testing.T) {
	tasksDir := t.TempDir()
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-list": "inv #1: active\n",
		},
	}
	app := newApp(exec, nil, nil)
	app.tasksDir = tasksDir
	app.busBin = "/bin/agentbus"
	app.busDir = "/tmp/bus"

	taskDir := filepath.Join(tasksDir, "worker-inv")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=worker-inv\nKIND=worker\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "worker-inv", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	if !strings.Contains(output, "== investigations") {
		t.Fatalf("output missing investigations header: %s", output)
	}
	if !strings.Contains(output, "inv #1: active") {
		t.Fatalf("output missing investigation details: %s", output)
	}
}

// TestCmdShowFinalMessageTrailingNewlines verifies trailing newlines in final.md are normalized.
func TestCmdShowFinalMessageTrailingNewlines(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	taskDir := filepath.Join(tasksDir, "final-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=final-task\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(taskDir, "final.md"), []byte("Done work.\n\n\n\n"), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "final-task", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	expected := "\n== the agent's last message\nDone work.\n"
	if !strings.Contains(output, expected) {
		t.Errorf("output does not contain cleanly normalized final message, got: %q", output)
	}
}
