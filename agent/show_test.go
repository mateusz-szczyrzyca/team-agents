package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// newShowTestApp returns an App instance configured with a temporary tasks directory.
func newShowTestApp(t *testing.T) (*App, string) {
	t.Helper()
	tasksDir := t.TempDir()
	return &App{
		Executor:  &appExecutor{},
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busDir:    t.TempDir(),
		agentHome: t.TempDir(),
	}, tasksDir
}

// createShowTaskDir creates a task directory under tasksDir with optional meta.env content.
func createShowTaskDir(t *testing.T, tasksDir, id, meta string) string {
	t.Helper()
	dir := filepath.Join(tasksDir, id)
	if err := os.MkdirAll(dir, 0750); err != nil {
		t.Fatal(err)
	}
	if meta != "" {
		if err := os.WriteFile(filepath.Join(dir, "meta.env"), []byte(meta), 0600); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

// TestCmdShowTaskNotFound verifies ShowNotFound error is returned.
func TestCmdShowTaskNotFound(t *testing.T) {
	app, _ := newShowTestApp(t)
	var out bytes.Buffer
	err := cmdShow(context.Background(), app, "nonexistent-task", &out)
	if err == nil {
		t.Fatal("cmdShow should return error for nonexistent task")
	}
	if err.Cause != ShowNotFound {
		t.Errorf("got error %v, want ShowNotFound", err)
	}
}

// TestCmdShowMetaBlankLines preserves interior and final blank metadata lines.
func TestCmdShowMetaBlankLines(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	createShowTaskDir(t, tasksDir, "show-test", "ID=show-test\n\nKIND=remote-member\n\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "show-test", &out); err != nil {
		t.Fatal(err)
	}
	if want := "== show-test\n  ID=show-test\n  \n  KIND=remote-member\n  \n"; out.String() != want {
		t.Fatalf("metadata output = %q, want %q", out.String(), want)
	}
}

// TestCmdShowMetaenv verifies meta.env is printed indented by 2 spaces.
func TestCmdShowMetaenv(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	createShowTaskDir(t, tasksDir, "show-test", "ID=show-test\nAGENT=test-agent\nSTATUS=done\nROUND=1\nRUN=1\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "show-test", &out); err != nil {
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
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "taskmd-test", "ID=taskmd-test\nSTATUS=done\nROUND=1\nRUN=1\n")
	task := "# Test Task\n\nThis is a test task description."
	if err := os.WriteFile(filepath.Join(taskDir, "task.md"), []byte(task), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "taskmd-test", &out); err != nil {
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
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "questions-test", "ID=questions-test\nSTATUS=done\nROUND=2\nRUN=1\n")
	questions := "# Questions for Round 2"
	if err := os.WriteFile(filepath.Join(taskDir, "questions-2.md"), []byte(questions), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "questions-test", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	output := out.String()
	if !strings.Contains(output, "questions (round 2)") {
		t.Errorf("output missing questions header: %s", output)
	}
}

// TestCmdShowFinalMessage verifies final message from final.md is shown.
func TestCmdShowFinalMessage(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "final-test", "ID=final-test\nSTATUS=done\nROUND=1\nRUN=1\n")
	final := "Final message from the agent"
	if err := os.WriteFile(filepath.Join(taskDir, "final.md"), []byte(final), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "final-test", &out); err != nil {
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
	app, tasksDir := newShowTestApp(t)
	createShowTaskDir(t, tasksDir, "worker-task", "ID=worker-task\nKIND=worker\nSTATUS=done\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "worker-task", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	output := out.String()
	if strings.Contains(output, "== investigations") {
		t.Errorf("worker task should not show empty investigations header: %s", output)
	}
}

// TestCmdShowWorktreeFromMeta verifies WORKTREE is read from meta.env, not app.workTree.
func TestCmdShowWorktreeFromMeta(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	appWorkTree := "/wrong/path"
	app.workTree = appWorkTree
	createShowTaskDir(t, tasksDir, "wt-task", "ID=wt-task\nWORKTREE=/correct/path\nSTATUS=done\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "wt-task", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	output := out.String()
	if strings.Contains(output, appWorkTree) && strings.Contains(output, "== report:") {
		t.Errorf("report path should use meta.env WORKTREE, not app.workTree: %s", output)
	}
}

// TestCmdShowQuestionsContent verifies questions header is printed without content.
func TestCmdShowQuestionsContent(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "q-task", "ID=q-task\nROUND=1\nSTATUS=done\n")
	questions := "# Questions - Round 1\n\n## 1. First question\n**Recommended:** a) Option A"
	if err := os.WriteFile(filepath.Join(taskDir, "questions-1.md"), []byte(questions), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "q-task", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	output := out.String()
	if !strings.Contains(output, "== questions") {
		t.Errorf("output missing questions header: %s", output)
	}
	if strings.Contains(output, "First question") {
		t.Errorf("output should not include questions content: %s", output)
	}
}

// TestCmdShowReport verifies report is shown from .agents/reports/$id.md.
func TestCmdShowReport(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	workTree := t.TempDir()
	app.workTree = workTree
	createShowTaskDir(t, tasksDir, "report-test", "ID=report-test\nSTATUS=done\nROUND=1\nRUN=1\n")
	reportDir := filepath.Join(workTree, ".agents", "reports")
	if err := os.MkdirAll(reportDir, 0750); err != nil {
		t.Fatal(err)
	}
	report := "# Task Report\n\nCompleted successfully."
	if err := os.WriteFile(filepath.Join(reportDir, "report-test.md"), []byte(report), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "report-test", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	output := out.String()
	if !strings.Contains(output, "== report:") {
		t.Errorf("output missing report header: %s", output)
	}
}

// TestCmdShowQuestionsHeaderWithoutFile verifies questions header prints when ROUND > 0 even if file does not exist.
func TestCmdShowQuestionsHeaderWithoutFile(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	createShowTaskDir(t, tasksDir, "q-missing", "ID=q-missing\nROUND=2\nSTATUS=done\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "q-missing", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	output := out.String()
	if !strings.Contains(output, "== questions (round 2):") {
		t.Errorf("output missing questions header: %s", output)
	}
}

// TestCmdShowTaskmdDirectory verifies ShowStorage error when task.md is a directory.
func TestCmdShowTaskmdDirectory(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "bad-task", "ID=bad-task\nSTATUS=done\n")
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

// TestCmdShowWorkerWithInvestigations verifies worker task prints investigations section when non-empty.
func TestCmdShowWorkerWithInvestigations(t *testing.T) {
	exec := &readCmdsTestExecutor{
		responses: map[string]string{
			"inv-list": "inv #1: active\n",
		},
	}
	tasksDir := t.TempDir()
	app := &App{
		Executor:  exec,
		tasksDir:  tasksDir,
		Clock:     appClock{},
		Random:    appRandom{},
		busBin:    "/bin/agentbus",
		busDir:    "/tmp/bus",
		agentHome: t.TempDir(),
	}
	createShowTaskDir(t, tasksDir, "worker-inv", "ID=worker-inv\nKIND=worker\nSTATUS=done\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "worker-inv", &out); err != nil {
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
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "final-task", "ID=final-task\nSTATUS=done\n")
	if err := os.WriteFile(filepath.Join(taskDir, "final.md"), []byte("Done work.\n\n\n\n"), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "final-task", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	expected := "\n== the agent's last message\nDone work.\n"
	if !strings.Contains(out.String(), expected) {
		t.Errorf("output does not contain cleanly normalized final message, got: %q", out.String())
	}
}

// TestCmdShowStorageFailures verifies that I/O failures for required and optional task files return ShowStorage.
func TestCmdShowStorageFailures(t *testing.T) {
	t.Run("unreadable meta.env returns ShowStorage", func(t *testing.T) {
		app, tasksDir := newShowTestApp(t)
		taskDir := filepath.Join(tasksDir, "unreadable-meta")
		if err := os.Mkdir(taskDir, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte("ID=unreadable-meta\n"), 0000); err != nil {
			t.Fatal(err)
		}
		var out bytes.Buffer
		err := cmdShow(context.Background(), app, "unreadable-meta", &out)
		if err == nil {
			t.Fatal("expected ShowStorage error, got nil")
		}
		if err.Cause != ShowStorage || !errors.Is(err, ErrStorage) {
			t.Fatalf("expected ShowStorage / ErrStorage, got %v", err)
		}
	})

	t.Run("unreadable task.md returns ShowStorage", func(t *testing.T) {
		app, tasksDir := newShowTestApp(t)
		taskDir := createShowTaskDir(t, tasksDir, "unreadable-taskmd", "ID=unreadable-taskmd\n")
		if err := os.WriteFile(filepath.Join(taskDir, "task.md"), []byte("task description\n"), 0000); err != nil {
			t.Fatal(err)
		}
		var out bytes.Buffer
		err := cmdShow(context.Background(), app, "unreadable-taskmd", &out)
		if err == nil {
			t.Fatal("expected ShowStorage error, got nil")
		}
		if err.Cause != ShowStorage || !errors.Is(err, ErrStorage) {
			t.Fatalf("expected ShowStorage / ErrStorage, got %v", err)
		}
	})
}

// TestCmdShowWorktreeReportRegression verifies that the report line uses WORKTREE from meta.env when a real report exists.
func TestCmdShowWorktreeReportRegression(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	appWorkTree := t.TempDir()
	metaWorkTree := t.TempDir()
	app.workTree = appWorkTree

	reportDir := filepath.Join(metaWorkTree, ".agents", "reports")
	if err := os.MkdirAll(reportDir, 0750); err != nil {
		t.Fatal(err)
	}
	reportFile := filepath.Join(reportDir, "wt-task.md")
	if err := os.WriteFile(reportFile, []byte("# Report"), 0600); err != nil {
		t.Fatal(err)
	}

	createShowTaskDir(t, tasksDir, "wt-task", fmt.Sprintf("ID=wt-task\nWORKTREE=%s\nSTATUS=done\n", metaWorkTree))
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "wt-task", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	wantLine := fmt.Sprintf("\n== report: %s\n", reportFile)
	if !strings.Contains(out.String(), wantLine) {
		t.Fatalf("output does not contain expected report line %q: %s", wantLine, out.String())
	}
}

// TestCmdShowFinalMessageTrailingNewlinesExact verifies that final.md trailing newlines are stripped exactly.
func TestCmdShowFinalMessageTrailingNewlinesExact(t *testing.T) {
	app, tasksDir := newShowTestApp(t)
	taskDir := createShowTaskDir(t, tasksDir, "trailing-task", "ID=trailing-task\n")
	if err := os.WriteFile(filepath.Join(taskDir, "final.md"), []byte("Message content\n\n\n"), 0600); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "trailing-task", &out); err != nil {
		t.Fatalf("cmdShow failed: %v", err)
	}
	if strings.HasSuffix(out.String(), "\n\n") {
		t.Fatalf("output has trailing empty lines: %q", out.String())
	}
	want := "\n== the agent's last message\nMessage content\n"
	if !strings.HasSuffix(out.String(), want) {
		t.Fatalf("output does not end with exactly %q: %q", want, out.String())
	}
}

// TestCmdShowWorkerInvListFailure verifies failing inv-list exits 0, writes no stderr, and logs inv_list_failed.
func TestCmdShowWorkerInvListFailure(t *testing.T) {
	exec := &readCmdsTestExecutor{
		exitCodes: map[string]int{"inv-list": 1},
		stderrs:   map[string]string{"inv-list": "simulated agentbus failure\n"},
	}
	tasksDir, agentHome := t.TempDir(), t.TempDir()
	var stderr bytes.Buffer
	app := &App{
		Executor:  exec,
		tasksDir:  tasksDir,
		busBin:    "/bin/agentbus",
		busDir:    "/tmp/bus",
		agentHome: agentHome,
		Stderr:    &stderr,
	}
	createShowTaskDir(t, tasksDir, "worker-fail", "ID=worker-fail\nKIND=worker\nSTATUS=done\n")
	var out bytes.Buffer
	if err := cmdShow(context.Background(), app, "worker-fail", &out); err != nil {
		t.Fatalf("cmdShow failed on inv-list failure: %v", err)
	}
	if stderr.Len() != 0 {
		t.Fatalf("expected empty stderr, got: %q", stderr.String())
	}
	logBytes, err := os.ReadFile(filepath.Join(agentHome, "logs", "agent.jsonl")) // #nosec G304 -- path belongs to temporary test directory
	if err != nil || !strings.Contains(string(logBytes), `"event":"inv_list_failed"`) {
		t.Fatalf("expected inv_list_failed log record, err=%v content=%s", err, string(logBytes))
	}
}
