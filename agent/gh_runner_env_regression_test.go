package main

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestDeriveGitHubParams_ColorsFailedWarning asserts structured warning emission, file field, and lock failure cause.
func TestDeriveGitHubParams_ColorsFailedWarning(t *testing.T) {
	tmp := t.TempDir()
	taskDir := filepath.Join(tmp, "tasks", "t1")
	if err := os.MkdirAll(taskDir, 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte("AGENT=a1\nTEAM=t1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	colorsPath := filepath.Join(tmp, "github-colors")
	if err := os.MkdirAll(colorsPath+".lock", 0o750); err != nil {
		t.Fatal(err)
	}
	cfg := newConfig()
	cfg.Scalars["AGENT_GITHUB_COLORS"] = "#112233"
	logger := &Logger{LogDir: filepath.Join(tmp, "logs"), Subcommand: "runner"}
	params := GitHubRunDerivationParams{
		AgentsHome: tmp, TasksDir: filepath.Join(tmp, "tasks"), BusDir: filepath.Join(tmp, "bus"),
		RunnerID: "t1", Config: cfg, Logger: logger,
	}
	if _, err := DeriveGitHubParams(context.Background(), params); err != nil {
		t.Fatalf("DeriveGitHubParams failed: %v", err)
	}
	logData, err := os.ReadFile(filepath.Clean(filepath.Join(tmp, "logs", "agent.jsonl"))) //nolint:gosec // test log path
	if err != nil {
		t.Fatalf("reading log failed: %v", err)
	}
	var rec struct {
		Event, File, Cause string
	}
	if err := json.Unmarshal(bytes.TrimSpace(logData), &rec); err != nil {
		t.Fatalf("json.Unmarshal(%s) failed: %v", logData, err)
	}
	if rec.Event != string(LogGitHubColorsFailed) || rec.File != colorsPath || !strings.Contains(rec.Cause, "github-colors.lock") {
		t.Errorf("unexpected record: %+v", rec)
	}
}

// TestDeriveGitHubParams_EmptyOrMissingRunnerID asserts graceful handling of empty or nonexistent RunnerID.
func TestDeriveGitHubParams_EmptyOrMissingRunnerID(t *testing.T) {
	tmp := t.TempDir()
	params := GitHubRunDerivationParams{
		AgentsHome: tmp, TasksDir: filepath.Join(tmp, "tasks"), BusDir: filepath.Join(tmp, "bus"),
		Config: newConfig(),
	}
	if _, err := DeriveGitHubParams(context.Background(), params); err != nil {
		t.Errorf("empty RunnerID failed: %v", err)
	}
	params.RunnerID = "nonexistent"
	if _, err := DeriveGitHubParams(context.Background(), params); err != nil {
		t.Errorf("missing RunnerID meta.env failed: %v", err)
	}
}
