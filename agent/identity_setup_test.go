package main

import (
	"context"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestSetupFormat tests Setup() with custom and default author formats.
func TestSetupFormat(t *testing.T) {
	id := &Identity{Agent: "bot"}

	// Empty format uses default
	env, err := id.Setup(context.Background(), "", nil)
	if err != nil {
		t.Fatalf("Setup with empty format failed: %v", err)
	}
	if env["GIT_AUTHOR_NAME"] != "bot (team-agents)" {
		t.Errorf("Empty format should use default, got %q", env["GIT_AUTHOR_NAME"])
	}

	// Custom format
	env, err = id.Setup(context.Background(), "%a [custom]", nil)
	if err != nil {
		t.Fatalf("Setup with custom format failed: %v", err)
	}
	if env["GIT_AUTHOR_NAME"] != "bot [custom]" {
		t.Errorf("Custom format failed, got %q", env["GIT_AUTHOR_NAME"])
	}
}

// TestSetupReadableKeyLogging tests logging with readable key.
func TestSetupReadableKeyLogging(t *testing.T) {
	tmpdir := t.TempDir()
	keyFile := filepath.Join(tmpdir, "key")
	if err := os.WriteFile(keyFile, []byte("fake"), 0600); err != nil {
		t.Fatalf("Failed to create key: %v", err)
	}

	logger := &Logger{LogDir: tmpdir}
	id := &Identity{Agent: "test", SigningKey: keyFile}
	env, err := id.Setup(context.Background(), "%a", logger)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}

	// Verify signing enabled
	if env["GIT_CONFIG_COUNT"] != "4" {
		t.Errorf("Readable key: GIT_CONFIG_COUNT=%q, want 4", env["GIT_CONFIG_COUNT"])
	}

	// Verify git_identity logged with signing=yes
	entries := readLogLines(t, filepath.Join(tmpdir, "agent.jsonl"))
	found := false
	for _, e := range entries {
		if e["event"] == "git_identity" && e["signing"] == "yes" {
			found = true
		}
	}
	if !found {
		t.Error("Expected git_identity with signing=yes in logs")
	}
}

// TestSetupUnreadableKeyLogging tests logging with unreadable key.
func TestSetupUnreadableKeyLogging(t *testing.T) {
	tmpdir := t.TempDir()
	logger := &Logger{LogDir: tmpdir}
	id := &Identity{Agent: "test", SigningKey: "/nonexistent/key"}
	env, err := id.Setup(context.Background(), "%a", logger)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}

	// Verify signing disabled
	if env["GIT_CONFIG_COUNT"] != "2" {
		t.Errorf("Unreadable key: GIT_CONFIG_COUNT=%q, want 2", env["GIT_CONFIG_COUNT"])
	}

	// Verify signing_key_unreadable warning logged
	entries := readLogLines(t, filepath.Join(tmpdir, "agent.jsonl"))
	found := false
	for _, e := range entries {
		if e["event"] == "signing_key_unreadable" {
			found = true
		}
	}
	if !found {
		t.Error("Expected signing_key_unreadable warning in logs")
	}
}

// TestSetupEnvironmentIndependence tests environment independence from mutations.
func TestSetupEnvironmentIndependence(t *testing.T) {
	id := &Identity{Agent: "test"}
	env1, err := id.Setup(context.Background(), "%a", nil)
	if err != nil {
		t.Fatalf("First Setup failed: %v", err)
	}

	// Record original
	orig := env1["GIT_AUTHOR_NAME"]

	// Mutate returned map
	env1["GIT_AUTHOR_NAME"] = "MUTATED"

	// Get new environment
	env2, err := id.Setup(context.Background(), "%a", nil)
	if err != nil {
		t.Fatalf("Second Setup failed: %v", err)
	}

	if env2["GIT_AUTHOR_NAME"] != orig {
		t.Error("Setup() returned maps not independent")
	}
}

// TestSetupProcessEnvIndependence tests environment doesn't leak os.Environ.
func TestSetupProcessEnvIndependence(t *testing.T) {
	// Set a unique process environment variable
	if err := os.Setenv("__CLAUDE_TEST_UNIQUE_VAR__", "should_not_appear"); err != nil {
		t.Fatalf("Failed to set env: %v", err)
	}

	id := &Identity{Agent: "test"}
	env, err := id.Setup(context.Background(), "%a", nil)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}

	// Verify environment doesn't contain process env vars
	if _, found := env["__CLAUDE_TEST_UNIQUE_VAR__"]; found {
		t.Error("Setup() leaked process environment variable")
	}
}

// readLogLines reads JSON log lines from a file and returns them as maps.
func readLogLines(t *testing.T, logfile string) []map[string]interface{} {
	data, err := os.ReadFile(logfile) // #nosec G304 - test-controlled path in temp directory
	if err != nil {
		return []map[string]interface{}{}
	}

	var entries []map[string]interface{}
	for _, line := range strings.Split(string(data), "\n") {
		if line == "" {
			continue
		}
		var e map[string]interface{}
		if err := json.Unmarshal([]byte(line), &e); err != nil {
			t.Logf("Failed to parse log line: %v", err)
			continue
		}
		entries = append(entries, e)
	}
	return entries
}

// TestSetupExistsButUnreadableKey tests SigningEnv readability handling.
// Regression: SigningEnv uses os.Stat (exists), Setup uses keyReadable (readable).
// File exists but unreadable should result in no signing config.
func TestSetupExistsButUnreadableKey(t *testing.T) {
	tmpdir := t.TempDir()
	keyFile := filepath.Join(tmpdir, "unreadable")
	if err := os.WriteFile(keyFile, []byte("key"), 0000); err != nil {
		t.Fatalf("Failed to create unreadable key: %v", err)
	}

	id := &Identity{Agent: "test", SigningKey: keyFile}
	env, err := id.Setup(context.Background(), "%a", nil)
	if err != nil {
		t.Fatalf("Setup failed: %v", err)
	}
	if env["GIT_CONFIG_COUNT"] != "2" {
		t.Errorf("Unreadable key should disable signing, got GIT_CONFIG_COUNT=%q, want 2", env["GIT_CONFIG_COUNT"])
	}
}

// TestSetupLoggerFailureNonfatal tests logger errors don't block Setup.
// Regression: Setup returns logger errors immediately (fatal).
// Logger errors should be logged as warnings but not block operation.
func TestSetupLoggerFailureNonfatal(t *testing.T) {
	tmpdir := t.TempDir()
	// Use read-only directory to force logger.Log to fail
	baddir := filepath.Join(tmpdir, "baddir")
	if err := os.Mkdir(baddir, 0000); err != nil {
		t.Fatalf("Failed to create read-only dir: %v", err)
	}
	failLogger := &Logger{LogDir: baddir}
	id := &Identity{Agent: "test", EmailConfig: "%a@example.com"}
	env, err := id.Setup(context.Background(), "%a", failLogger)

	// Logger failure should NOT be returned as Setup error (nonfatal)
	if err != nil {
		t.Errorf("Logger failure should be nonfatal, but Setup returned error: %v", err)
	}
	// Environment should still be populated
	if env["GIT_AUTHOR_EMAIL"] != "test@example.com" {
		t.Errorf("Environment should be populated despite logger failure")
	}
}

// TestSetupLogsIdentityDespiteRunnerFailure tests that git_identity is logged to healthy sinks (agent.jsonl)
// when the runner sink (fd 3) fails. The warning and identity records must both be present in healthy sinks.
func TestSetupLogsIdentityDespiteRunnerFailure(t *testing.T) {
	tmpdir := t.TempDir()
	logDir := filepath.Join(tmpdir, "logs")
	failRunner := &failAfterCount{limit: 0} // Fail all writes
	logger := &Logger{LogDir: logDir, Runner: failRunner, Task: "test"}
	id := &Identity{Agent: "test", SigningKey: "/nonexistent/key"}
	id.Setup(context.Background(), "", logger)
	// Both warning and git_identity should be logged to agent.jsonl despite runner failure
	data, err := os.ReadFile(filepath.Join(logDir, "agent.jsonl"))
	if err != nil {
		t.Fatalf("agent.jsonl: %v", err)
	}
	content := string(data)
	if !strings.Contains(content, `"event":"signing_key_unreadable"`) {
		t.Error("warning not logged to agent.jsonl")
	}
	if !strings.Contains(content, `"event":"git_identity"`) {
		t.Error("git_identity not logged to agent.jsonl despite runner failure")
	}
}

// failAfterCount fails after a limit of successful writes.
type failAfterCount struct {
	limit int
	count int
}

func (f *failAfterCount) Write(p []byte) (int, error) {
	if f.count < f.limit {
		f.count++
		return len(p), nil
	}
	return 0, io.EOF
}
