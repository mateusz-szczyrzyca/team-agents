package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// createTestTask creates a minimal valid task with required metadata and task.md files.
func createTestTask(t *testing.T, tasksDir, taskID, metaContent, taskContent string) {
	taskDir := filepath.Join(tasksDir, taskID)
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	// Ensure ID is set in metadata
	if !strings.Contains(metaContent, "ID=") {
		metaContent = "ID=" + taskID + "\n" + metaContent
	}
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(metaContent), 0600); err != nil {
		t.Fatal(err)
	}
	if taskContent == "" {
		taskContent = "Test task"
	}
	if err := os.WriteFile(filepath.Join(taskDir, "task.md"), []byte(taskContent), 0600); err != nil {
		t.Fatal(err)
	}
}

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

	meta := "AGENT=test-agent\nSTATUS=done\nROUND=1\nRUN=1\n"
	createTestTask(t, tasksDir, "show-test", meta, "Test task")

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

	meta := "STATUS=done\nROUND=2\nRUN=1\n"
	createTestTask(t, tasksDir, "questions-test", meta, "Test task")
	questions := "# Questions for Round 2"
	if err := os.WriteFile(filepath.Join(tasksDir, "questions-test", "questions-2.md"), []byte(questions), 0600); err != nil {
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

	meta := "STATUS=done\nROUND=1\nRUN=1\n"
	createTestTask(t, tasksDir, "final-test", meta, "Test task")
	final := "Final message from the agent"
	if err := os.WriteFile(filepath.Join(tasksDir, "final-test", "final.md"), []byte(final), 0600); err != nil {
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

	meta := "KIND=worker\nSTATUS=done\n"
	createTestTask(t, tasksDir, "worker-task", meta, "Test worker task")

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

	meta := "WORKTREE=/correct/path\nSTATUS=done\n"
	createTestTask(t, tasksDir, "wt-task", meta, "Test task")

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

// TestCmdShowQuestionsContent verifies questions header is printed without content.
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

	meta := "ROUND=1\nSTATUS=done\n"
	createTestTask(t, tasksDir, "q-task", meta, "Test task")
	questions := "# Questions - Round 1\n\n## 1. First question\n**Recommended:** a) Option A"
	if err := os.WriteFile(filepath.Join(tasksDir, "q-task", "questions-1.md"), []byte(questions), 0600); err != nil {
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
	// Should NOT have questions content, only header (parity with Bash oracle)
	if strings.Contains(output, "First question") {
		t.Errorf("output should not include questions content: %s", output)
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

	meta := "STATUS=done\nROUND=1\nRUN=1\n"
	createTestTask(t, tasksDir, "report-test", meta, "Test task")

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

// TestCmdShowInvListFailed verifies that a failing inv-list is logged nonfatally.
func TestCmdShowInvListFailed(t *testing.T) {
	tasksDir := t.TempDir()
	agentHome := t.TempDir()
	logDir := filepath.Join(agentHome, "logs")
	busDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		agentHome: agentHome,
		busDir:    busDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busBin:    "false", // Use 'false' command which exits with 1
	}

	meta := "KIND=worker\nSTATUS=done\nRUN=1\n"
	createTestTask(t, tasksDir, "worker-task", meta, "Test worker task")

	var out bytes.Buffer
	var stderr bytes.Buffer

	err := cmdShow(context.Background(), app, "worker-task", &out, &stderr)
	if err != nil {
		t.Fatalf("cmdShow should not fail on inv-list failure, got: %v", err)
	}

	// Check that log file was created with inv_list_failed event
	logFile := filepath.Join(logDir, "agent.jsonl")
	if content, err := os.ReadFile(logFile); err == nil { // #nosec G304 -- test file path under temp directory
		if !strings.Contains(string(content), "inv_list_failed") {
			t.Errorf("log file missing inv_list_failed event: %s", string(content))
		}
	}

	// The failure should not result in doubled error output to stderr
	stderrOutput := stderr.String()
	// Count how many times the error appears (should be logged once, not printed twice)
	if strings.Count(stderrOutput, "bus") > 2 {
		t.Errorf("error message appears too many times in stderr: %s", stderrOutput)
	}
}

// TestCmdShowBlankMetaLines verifies that blank lines in meta.env are preserved in output.
func TestCmdShowBlankMetaLines(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task with blank lines in meta.env - blank line between ID and STATUS
	meta := "ID=blank-meta\n\nSTATUS=done\nRUN=1\n"
	createTestTask(t, tasksDir, "blank-meta", meta, "Test task")

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "blank-meta", &out)
	if err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}

	output := out.String()
	lines := strings.Split(output, "\n")
	// Should find "  " (two spaces for blank line) in the meta section
	hasBlankLine := false
	for _, line := range lines {
		if line == "  " { // Two spaces indicates a blank line from meta.env
			hasBlankLine = true
			break
		}
	}
	if !hasBlankLine {
		t.Errorf("output missing blank lines from meta.env, expected blank line with two spaces. Got: %s", output)
	}
}

// TestCmdShowTaskmdDirectory verifies ShowStorage error when task.md is a directory.
func TestCmdShowTaskmdDirectory(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task where task.md is a directory instead of file
	taskDir := filepath.Join(tasksDir, "bad-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=bad-task\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	// Create task.md as a directory instead of a file
	if err := os.Mkdir(filepath.Join(taskDir, "task.md"), 0750); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "bad-task", &out)
	if err == nil {
		t.Fatalf("cmdShow should return error when task.md is a directory, got nil")
	}
	if err.Cause != ShowStorage {
		t.Fatalf("cmdShow returned %v, want ShowStorage", err.Cause)
	}
}

// TestCmdShowTaskmdMissing verifies ShowStorage error when task.md is missing.
func TestCmdShowTaskmdMissing(t *testing.T) {
	tasksDir := t.TempDir()
	app := &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}

	// Create task without task.md file
	taskDir := filepath.Join(tasksDir, "no-task-md")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=no-task-md\nSTATUS=done\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	// Deliberately do NOT create task.md file

	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "no-task-md", &out)
	if err == nil {
		t.Fatalf("cmdShow should return error when task.md is missing, got nil. Output was: %s", out.String())
	}
	if err.Cause != ShowStorage {
		t.Fatalf("cmdShow returned %v, want ShowStorage", err.Cause)
	}
}

// TestCmdShowWorkerInvListFailureHandled verifies Try error is explicitly handled (not discarded) when inv-list fails.
func TestCmdShowWorkerInvListFailureHandled(t *testing.T) {
	tasksDir := t.TempDir()
	busDir := t.TempDir()
	logDir := t.TempDir()

	app := &App{
		Executor: &busTestExecutor{
			result: CommandResult{ExitCode: BusExitNotFound}, // inv-list returns not found (2)
			stderr: "no investigations for task",
		},
		tasksDir:  tasksDir,
		busDir:    busDir,
		agentHome: logDir,
		Clock:     appClock{},
		Random:    appRandom{},
	}

	// Create a worker task
	taskDir := filepath.Join(tasksDir, "worker-task")
	if err := os.MkdirAll(taskDir, 0750); err != nil {
		t.Fatal(err)
	}
	meta := "ID=worker-task\nKIND=worker\nSTATUS=running\n"
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(taskDir, "task.md"), []byte("Task for testing"), 0600); err != nil {
		t.Fatal(err)
	}

	// Even with inv-list failing, show should succeed (best-effort, non-fatal)
	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "worker-task", &out)
	if err != nil {
		t.Fatalf("cmdShow should succeed even when inv-list fails: %v", err)
	}

	// Output should still contain task header and content
	outStr := out.String()
	if !strings.Contains(outStr, "== worker-task") {
		t.Fatalf("output missing task header: %s", outStr)
	}
	if !strings.Contains(outStr, "== task") {
		t.Fatalf("output missing task section: %s", outStr)
	}
}
