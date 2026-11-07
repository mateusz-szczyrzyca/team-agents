package main

import (
	"context"
	"errors"
	"os"
	execpkg "os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// TestSetupGitHubRun_MissingGH verifies that missing gh binary emits a warning, succeeds as no-op,
// and does not create the wrapper directory or modify PATH.
func TestSetupGitHubRun_MissingGH(t *testing.T) {
	ctx := context.Background()
	tmpDir := t.TempDir()
	logDir := filepath.Join(tmpDir, "logs")
	logger := &Logger{LogDir: logDir, Subcommand: "runner"}

	params := GitHubSetupParams{
		AgentsHome:     tmpDir,
		SelfExecutable: "/bin/agent",
		RepoRoot:       tmpDir,
		RunnerID:       "task-missing-gh",
		CurrentPath:    "/usr/bin:/bin",
		LookPath: func(string) (string, error) {
			return "", execpkg.ErrNotFound
		},
		Logger: logger,
	}

	exec := &fakeGHExecutor{}
	runEnv, err := SetupGitHubRun(ctx, exec, params)
	if err != nil {
		t.Fatalf("SetupGitHubRun with missing gh failed: %v", err)
	}
	if runEnv == nil {
		t.Fatal("SetupGitHubRun returned nil runEnv")
	}

	wrapperDir := filepath.Join(tmpDir, "run", "gh-trusted")
	if _, statErr := os.Stat(wrapperDir); !os.IsNotExist(statErr) {
		t.Errorf("wrapper dir %s was created, but missing gh must not create it", wrapperDir)
	}
	if runEnv.EffectivePath != params.CurrentPath {
		t.Errorf("EffectivePath = %q; want %q", runEnv.EffectivePath, params.CurrentPath)
	}
	if runEnv.PrependPath != "" {
		t.Errorf("PrependPath = %q; want empty string", runEnv.PrependPath)
	}

	logFile := filepath.Clean(filepath.Join(logDir, "agent.jsonl"))
	data, readErr := os.ReadFile(logFile)
	if readErr != nil {
		t.Fatalf("failed to read log file: %v", readErr)
	}
	if !strings.Contains(string(data), string(LogAgentsGHMissing)) {
		t.Errorf("log file does not contain %s event: %s", LogAgentsGHMissing, string(data))
	}
	if !strings.Contains(string(data), "task=task-missing-gh") {
		t.Errorf("log file missing task field: %s", string(data))
	}
}

// TestSetupGitHubRun_PathAlreadyContainsWrapper verifies PATH is not double-prepended.
func TestSetupGitHubRun_PathAlreadyContainsWrapper(t *testing.T) {
	ctx := context.Background()
	tmpDir := t.TempDir()
	selfBin := filepath.Join(tmpDir, "agent-bin")
	if err := os.WriteFile(selfBin, []byte("#!/bin/sh\nexit 0\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	wrapperDir := filepath.Join(tmpDir, "run", "gh-trusted")
	existingPath := wrapperDir + ":/usr/bin:/bin"

	params := GitHubSetupParams{
		AgentsHome:     tmpDir,
		SelfExecutable: selfBin,
		RepoRoot:       tmpDir,
		RunnerID:       "task-path-check",
		CurrentPath:    existingPath,
		LookPath: func(string) (string, error) {
			return "/usr/bin/gh", nil
		},
		GitHubTrusted: "owner",
	}

	exec := &fakeGHExecutor{}
	runEnv, err := SetupGitHubRun(ctx, exec, params)
	if err != nil {
		t.Fatalf("SetupGitHubRun failed: %v", err)
	}
	if runEnv == nil {
		t.Fatal("SetupGitHubRun returned nil runEnv")
	}
	if runEnv.EffectivePath != existingPath {
		t.Errorf("EffectivePath = %q; want unchanged %q", runEnv.EffectivePath, existingPath)
	}
}

// TestSetupGitHubRun_ExactEnvAndGlobalUnchanged verifies full exported env and zero global env mutations.
func TestSetupGitHubRun_ExactEnvAndGlobalUnchanged(t *testing.T) {
	ctx := context.Background()
	tmpDir := t.TempDir()
	selfBin := filepath.Join(tmpDir, "agent-bin")
	if err := os.WriteFile(selfBin, []byte("#!/bin/sh\nexit 0\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	beforeEnv := os.Environ()

	params := GitHubSetupParams{
		AgentsHome:       tmpDir,
		SelfExecutable:   selfBin,
		RepoRoot:         tmpDir,
		RunnerID:         "task-456",
		Team:             "team-beta",
		Agent:            "codex-1",
		Coordinator:      "gemini-1",
		OwnPR:            "100",
		BusDir:           filepath.Join(tmpDir, "bus"),
		AgentbusBin:      filepath.Join(tmpDir, "bin", "agentbus"),
		ReviewByline:     "Review by investigator",
		GitHubTrusted:    "custom-owner custom-user",
		GitHubColors:     "#123456",
		GitHubNameColors: "codex-1=#123456",
		CurrentPath:      "/usr/bin:/bin",
		LookPath: func(string) (string, error) {
			return "/usr/local/bin/gh", nil
		},
	}

	exec := &fakeGHExecutor{}
	runEnv, err := SetupGitHubRun(ctx, exec, params)
	if err != nil {
		t.Fatalf("SetupGitHubRun failed: %v", err)
	}
	if runEnv == nil {
		t.Fatal("SetupGitHubRun returned nil runEnv")
	}

	afterEnv := os.Environ()
	if !reflect.DeepEqual(beforeEnv, afterEnv) {
		t.Error("SetupGitHubRun mutated global process environment")
	}

	expectedKeys := []string{
		"AGENT_REAL_GH",
		"AGENT_GITHUB_TRUSTED_RESOLVED",
		"AGENT_REVIEW_BYLINE",
		"AGENT_GITHUB_COLORS",
		"AGENT_GH_COORDINATOR",
		"AGENT_GH_TEAM",
		"AGENT_GH_SELF",
		"AGENT_GH_BUS_DIR",
		"AGENTBUS_BIN",
		"AGENT_GH_OWN_PR",
		"AGENT_GITHUB_NAME_COLORS",
	}

	for _, key := range expectedKeys {
		if _, ok := runEnv.Env[key]; !ok {
			t.Errorf("missing expected environment key: %s", key)
		}
	}

	if runEnv.Env["AGENT_REAL_GH"] != "/usr/local/bin/gh" {
		t.Errorf("AGENT_REAL_GH = %q; want /usr/local/bin/gh", runEnv.Env["AGENT_REAL_GH"])
	}
	if runEnv.Env["AGENT_GITHUB_TRUSTED_RESOLVED"] != "custom-owner custom-user" {
		t.Errorf("AGENT_GITHUB_TRUSTED_RESOLVED = %q; want 'custom-owner custom-user'", runEnv.Env["AGENT_GITHUB_TRUSTED_RESOLVED"])
	}
}

// TestResolveGitHubTrustedRaw_OracleSeparatorAndCaching verifies oracle printf format and per-process caching.
func TestResolveGitHubTrustedRaw_OracleSeparatorAndCaching(t *testing.T) {
	ctx := context.Background()
	ResetGHLoginCacheForTest()

	// Case 1: owner found, gh login fails -> returns "owner " with trailing space per oracle printf '%s %s'
	execOwnerOnly := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghErr:     errors.New("not logged in"),
	}
	raw := ResolveGitHubTrustedRaw(ctx, execOwnerOnly, "/dummy", "")
	expected := "mateusz-szczyrzyca "
	if raw != expected {
		t.Errorf("ResolveGitHubTrustedRaw = %q; want %q", raw, expected)
	}

	// Case 2: login succeeds on first call and is cached
	ResetGHLoginCacheForTest()
	execBoth := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "cached-user",
	}
	raw2 := ResolveGitHubTrustedRaw(ctx, execBoth, "/dummy", "")
	expected2 := "mateusz-szczyrzyca cached-user"
	if raw2 != expected2 {
		t.Errorf("ResolveGitHubTrustedRaw = %q; want %q", raw2, expected2)
	}

	// Calling again with executor that would fail if called: cached login is retained
	execFails := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghErr:     errors.New("should not be called due to cache"),
	}
	raw3 := ResolveGitHubTrustedRaw(ctx, execFails, "/dummy", "")
	if raw3 != expected2 {
		t.Errorf("ResolveGitHubTrustedRaw cached = %q; want %q", raw3, expected2)
	}
}

// TestEnsureGHWrapper_StorageFailure verifies typed storage error on directory creation failure.
func TestEnsureGHWrapper_StorageFailure(t *testing.T) {
	tmpDir := t.TempDir()
	// Create a regular file where the directory needs to be
	conflictPath := filepath.Join(tmpDir, "run")
	if err := os.WriteFile(conflictPath, []byte("blocker"), 0o600); err != nil {
		t.Fatal(err)
	}

	_, err := EnsureGHWrapper(tmpDir, "/bin/agent")
	if err == nil {
		t.Fatal("EnsureGHWrapper succeeded unexpectedly on path conflict")
	}
	if err.Cause != GitHubSetupStorage {
		t.Errorf("EnsureGHWrapper error cause = %v; want GitHubSetupStorage", err.Cause)
	}
}

// TestExtractGitHubOwner_InvalidOrNonGitHub verifies non-matching URLs return empty string.
func TestExtractGitHubOwner_InvalidOrNonGitHub(t *testing.T) {
	cases := []string{
		"",
		"https://gitlab.com/owner/repo.git",
		"invalid-url",
		"ssh://git@bitbucket.org/team/repo.git",
	}
	for _, c := range cases {
		if got := ExtractGitHubOwner(c); got != "" {
			t.Errorf("ExtractGitHubOwner(%q) = %q; want empty string", c, got)
		}
	}
}

// TestEnsureGHWrapper_RenameFailureCleanup verifies temp symlink cleanup and typed storage error on rename failure.
func TestEnsureGHWrapper_RenameFailureCleanup(t *testing.T) {
	tmpDir := t.TempDir()
	trustedDir := filepath.Join(tmpDir, "run", "gh-trusted")
	if err := os.MkdirAll(trustedDir, 0o750); err != nil {
		t.Fatal(err)
	}
	wrapperPath := filepath.Join(trustedDir, "gh")
	if err := os.Mkdir(wrapperPath, 0o750); err != nil {
		t.Fatal(err)
	}

	_, err := EnsureGHWrapper(tmpDir, "/bin/agent")
	if err == nil {
		t.Fatal("EnsureGHWrapper succeeded unexpectedly when destination is a directory")
	}
	if err.Cause != GitHubSetupStorage {
		t.Errorf("EnsureGHWrapper error cause = %v; want GitHubSetupStorage", err.Cause)
	}
	entries, readErr := os.ReadDir(trustedDir)
	if readErr != nil {
		t.Fatal(readErr)
	}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), "gh.") {
			t.Errorf("temporary symlink was not cleaned up: %s", entry.Name())
		}
	}
}

// TestSetupGitHubRun_LoggerFailurePropagated verifies Logger.Log failure returns GitHubSetupStorage.
func TestSetupGitHubRun_LoggerFailurePropagated(t *testing.T) {
	ctx := context.Background()
	tmpDir := t.TempDir()
	unwriteableLogDir := filepath.Join(tmpDir, "unwriteable-logs")
	if err := os.WriteFile(unwriteableLogDir, []byte("blocker"), 0o400); err != nil {
		t.Fatal(err)
	}
	logger := &Logger{LogDir: unwriteableLogDir, Subcommand: "runner"}

	params := GitHubSetupParams{
		AgentsHome:     tmpDir,
		SelfExecutable: "/bin/agent",
		RepoRoot:       tmpDir,
		RunnerID:       "task-log-fail",
		CurrentPath:    "/usr/bin:/bin",
		LookPath: func(string) (string, error) {
			return "", execpkg.ErrNotFound
		},
		Logger: logger,
	}

	exec := &fakeGHExecutor{}
	_, err := SetupGitHubRun(ctx, exec, params)
	if err == nil {
		t.Fatal("SetupGitHubRun succeeded unexpectedly when Logger.Log failed")
	}
	if err.Cause != GitHubSetupStorage {
		t.Errorf("SetupGitHubRun error cause = %v; want GitHubSetupStorage", err.Cause)
	}
}

// TestResolveGitHubTrustedRaw_WhitespaceOverridePreserved verifies nonempty whitespace override is preserved.
func TestResolveGitHubTrustedRaw_WhitespaceOverridePreserved(t *testing.T) {
	ctx := context.Background()
	exec := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "user",
	}
	got := ResolveGitHubTrustedRaw(ctx, exec, "/dummy", "   ")
	if got != "   " {
		t.Errorf("ResolveGitHubTrustedRaw with whitespace override = %q; want %q", got, "   ")
	}
}

// TestResolveGitHubTrustedRaw_ContextCacheIsolation verifies cache is isolated to WithGHLoginCache context.
func TestResolveGitHubTrustedRaw_ContextCacheIsolation(t *testing.T) {
	ctx1 := WithGHLoginCache(context.Background())
	ctx2 := WithGHLoginCache(context.Background())

	exec1 := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "user-ctx1",
	}
	raw1 := ResolveGitHubTrustedRaw(ctx1, exec1, "/dummy", "")
	if raw1 != "mateusz-szczyrzyca user-ctx1" {
		t.Errorf("ctx1 first call = %q; want 'mateusz-szczyrzyca user-ctx1'", raw1)
	}

	exec2 := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "user-ctx2",
	}
	raw2 := ResolveGitHubTrustedRaw(ctx2, exec2, "/dummy", "")
	if raw2 != "mateusz-szczyrzyca user-ctx2" {
		t.Errorf("ctx2 call = %q; want 'mateusz-szczyrzyca user-ctx2'", raw2)
	}
}

// TestResolveGitHubTrusted_ContextCacheReuse verifies repeated calls on the same context reuse the cached login.
func TestResolveGitHubTrusted_ContextCacheReuse(t *testing.T) {
	ctx := WithGHLoginCache(context.Background())
	exec := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "initial-user",
	}
	raw1, err1 := ResolveGitHubTrusted(ctx, exec, "/dummy", "")
	if err1 != nil || raw1 != "mateusz-szczyrzyca initial-user" {
		t.Fatalf("first call = (%q, %v); want ('mateusz-szczyrzyca initial-user', nil)", raw1, err1)
	}

	exec.ghUser = "changed-user"
	raw2, err2 := ResolveGitHubTrusted(ctx, exec, "/dummy", "")
	if err2 != nil || raw2 != "mateusz-szczyrzyca initial-user" {
		t.Errorf("second call with changed executor = (%q, %v); want cached 'mateusz-szczyrzyca initial-user'", raw2, err2)
	}
}
