package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

// TestDeriveGitHubParams_FirstRunRegistersAndFormats verifies that on the first run,
// before github-colors exists, RegisterGitHubColors runs before FormatGitHubNameColors,
// so that the returned GitHubNameColors includes the newly registered assignment.
func TestDeriveGitHubParams_FirstRunRegistersAndFormats(t *testing.T) {
	tmp := t.TempDir()
	tasksDir := filepath.Join(tmp, "tasks")
	taskID := "task-first-run"
	metaDir := filepath.Join(tasksDir, taskID)
	if err := os.MkdirAll(metaDir, 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(metaDir, "meta.env"), []byte("AGENT=first-agent\nTEAM=team-run\nKIND=worker\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	leadLookup := func(_ context.Context, _ string) (string, *ProcessError) {
		return "lead-agent", nil
	}

	cfg := newConfig()
	cfg.Scalars["AGENT_GITHUB_COLORS"] = "#0969DA #8250DF"
	params := GitHubRunDerivationParams{
		AgentsHome:     tmp,
		TasksDir:       tasksDir,
		BusDir:         filepath.Join(tmp, "bus"),
		RunnerID:       taskID,
		Config:         cfg,
		TeamLeadLookup: leadLookup,
	}

	setupParams, err := DeriveGitHubParams(context.Background(), params)
	if err != nil {
		t.Fatalf("DeriveGitHubParams failed: %v", err)
	}

	// first-agent should be assigned and formatted in GitHubNameColors immediately
	if !strings.Contains(setupParams.GitHubNameColors, "first-agent=") {
		t.Errorf("setupParams.GitHubNameColors = %q, expected to contain first-agent assignment on first run", setupParams.GitHubNameColors)
	}
}

// TestGitHubColors_PaletteNone verifies that palette 'none' disables both registration mutation
// and formatted name colors output.
func TestGitHubColors_PaletteNone(t *testing.T) {
	tmp := t.TempDir()
	colorsPath := filepath.Join(tmp, "github-colors")
	paletteNone := []string{"none"}

	// Registration with palette "none" must not write any file or allocate colors.
	err := RegisterGitHubColors(context.Background(), colorsPath, paletteNone, []string{"agent-1"}, nil)
	if err != nil {
		t.Fatalf("RegisterGitHubColors returned error: %v", err)
	}
	if _, statErr := os.Stat(colorsPath); !os.IsNotExist(statErr) {
		t.Errorf("colorsPath was created despite palette none: %v", statErr)
	}

	// FormatGitHubNameColors with palette "none" must return empty string.
	cfg := newConfig()
	cfg.Scalars["AGENT_GITHUB_COLORS"] = "none"
	cfg.Indexed["NORMAL_AGENTS"] = map[string]string{"0": "agent-1"}
	out := FormatGitHubNameColors(colorsPath, paletteNone, cfg)
	if out != "" {
		t.Errorf("FormatGitHubNameColors() = %q, want empty string for palette none", out)
	}
}

// TestReadColorLines_UnreadableError verifies that readColorLines reports a storage error
// when the file exists but cannot be read due to file permissions.
func TestReadColorLines_UnreadableError(t *testing.T) {
	tmp := t.TempDir()
	colorsPath := filepath.Join(tmp, "unreadable-colors")
	if err := os.WriteFile(colorsPath, []byte("agent-1 #0969DA\n"), 0o000); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = os.Chmod(colorsPath, 0o600)
	})

	// Probe whether the OS/user permits chmod 0000 read denial (skip if running as root)
	if _, readErr := os.ReadFile(filepath.Clean(colorsPath)); readErr == nil { //nolint:gosec // test path
		t.Skip("environment allows reading 0000 files (running as root)")
	}

	_, storageErr := readColorLinesChecked(colorsPath)
	if storageErr == nil {
		t.Fatal("expected StorageError for unreadable colors file, got nil")
	}
}

// TestRegisterGitHubColors_ContextCancellation verifies that context cancellation immediately
// halts the lock acquisition without blocking.
func TestRegisterGitHubColors_ContextCancellation(t *testing.T) {
	tmp := t.TempDir()
	colorsPath := filepath.Join(tmp, "github-colors")
	palette := []string{"#0969DA"}

	ctx, cancel := context.WithCancel(context.Background())
	cancel() // canceled before calling

	err := RegisterGitHubColors(ctx, colorsPath, palette, []string{"agent-1"}, nil)
	if err == nil {
		t.Fatal("expected StorageError on canceled context, got nil")
	}
}

// TestRegisterGitHubColors_ContentionTimeout verifies that lock acquisition respects timeout
// when the lock is held by another process/handle.
func TestRegisterGitHubColors_ContentionTimeout(t *testing.T) {
	tmp := t.TempDir()
	colorsPath := filepath.Join(tmp, "github-colors")
	lockPath := colorsPath + ".lock"
	if err := os.MkdirAll(tmp, 0o750); err != nil {
		t.Fatal(err)
	}
	lockFile, err := os.OpenFile(filepath.Clean(lockPath), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if closeErr := lockFile.Close(); closeErr != nil {
			t.Error(closeErr)
		}
	}()

	if err := syscall.Flock(int(lockFile.Fd()), syscall.LOCK_EX); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if unlockErr := syscall.Flock(int(lockFile.Fd()), syscall.LOCK_UN); unlockErr != nil {
			t.Error(unlockErr)
		}
	}()

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
	defer cancel()

	lockErr := RegisterGitHubColors(ctx, colorsPath, []string{"#0969DA"}, []string{"agent-2"}, nil)
	if lockErr == nil {
		t.Fatal("expected StorageError on lock contention, got nil")
	}
}
