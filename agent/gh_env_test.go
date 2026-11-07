package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

// fakeGHExecutor simulates git and gh CLI calls for testing trusted account resolution.
type fakeGHExecutor struct {
	originURL string
	ghUser    string
	ghErr     error
	gitErr    error
}

// Execute handles git and gh command execution for tests.
func (f *fakeGHExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	if cmd.Name == "git" {
		if f.gitErr != nil {
			return CommandResult{ExitCode: 1}, fromProcess(ExecuteStart, processErr("git", f.gitErr, "simulate git error"))
		}
		if cmd.Stdout != nil {
			if _, err := cmd.Stdout.Write([]byte(f.originURL + "\n")); err != nil {
				return CommandResult{ExitCode: 1}, fromProcess(ExecuteWait, processErr("git", err, "write stdout"))
			}
		}
		return CommandResult{ExitCode: 0}, nil
	}
	if cmd.Name == "gh" {
		if f.ghErr != nil {
			return CommandResult{ExitCode: 1}, fromProcess(ExecuteStart, processErr("gh", f.ghErr, "simulate gh error"))
		}
		if cmd.Stdout != nil {
			if _, err := cmd.Stdout.Write([]byte(f.ghUser + "\n")); err != nil {
				return CommandResult{ExitCode: 1}, fromProcess(ExecuteWait, processErr("gh", err, "write stdout"))
			}
		}
		return CommandResult{ExitCode: 0}, nil
	}
	return CommandResult{ExitCode: 127}, fromProcess(ExecuteInvalidCommand, processErr(cmd.Name, errors.New("unexpected command"), "not configured"))
}

// TestExtractGitHubOwner verifies parsing of GitHub repository owner from various URL formats.
func TestExtractGitHubOwner(t *testing.T) {
	cases := []struct {
		url      string
		expected string
	}{
		{"https://github.com/mateusz-szczyrzyca/team-agents.git", "mateusz-szczyrzyca"},
		{"https://github.com/octocat/hello-world", "octocat"},
		{"git@github.com:example-org/repo.git", "example-org"},
		{"ssh://git@github.com/deepmind/antigravity.git", "deepmind"},
		{"https://gitlab.com/other-org/other-repo.git", ""},
		{"invalid-url", ""},
		{"", ""},
	}

	for _, tc := range cases {
		actual := ExtractGitHubOwner(tc.url)
		if actual != tc.expected {
			t.Errorf("ExtractGitHubOwner(%q) = %q; want %q", tc.url, actual, tc.expected)
		}
	}
}

// TestResolveRealGH verifies resolution of the real gh binary and recursion avoidance.
func TestResolveRealGH(t *testing.T) {
	agentsHome := filepath.Join("/tmp", "agents-home-test")
	wrapperDir := filepath.Join(agentsHome, "run", "gh-trusted")
	wrapperPath := filepath.Join(wrapperDir, "gh")

	// Case 1: normal lookup succeeds.
	mockLookup := func(name string) (string, error) {
		if name == "gh" {
			return "/usr/bin/gh", nil
		}
		return "", errors.New("not found")
	}
	realPath, found, err := ResolveRealGH(mockLookup, agentsHome, "")
	if err != nil || !found || realPath != "/usr/bin/gh" {
		t.Fatalf("ResolveRealGH normal lookup = (%q, %v, %v); want (/usr/bin/gh, true, nil)", realPath, found, err)
	}

	// Case 2: lookup fails (gh missing).
	failLookup := func(_ string) (string, error) {
		return "", errors.New("executable file not found in $PATH")
	}
	realPath, found, err = ResolveRealGH(failLookup, agentsHome, "")
	if err != nil || found || realPath != "" {
		t.Fatalf("ResolveRealGH missing gh = (%q, %v, %v); want ('', false, nil)", realPath, found, err)
	}

	// Case 3: lookup points to wrapper itself, but currentRealGH is set.
	wrapperLookup := func(_ string) (string, error) {
		return wrapperPath, nil
	}
	realPath, found, err = ResolveRealGH(wrapperLookup, agentsHome, "/usr/local/bin/gh")
	if err != nil || !found || realPath != "/usr/local/bin/gh" {
		t.Fatalf("ResolveRealGH wrapper with env = (%q, %v, %v); want (/usr/local/bin/gh, true, nil)", realPath, found, err)
	}

	// Case 4: lookup points to wrapper, but currentRealGH is empty -> fails recursion check.
	realPath, found, err = ResolveRealGH(wrapperLookup, agentsHome, "")
	if err == nil || found {
		t.Fatalf("ResolveRealGH recursive wrapper without fallback = (%q, %v, %v); want error", realPath, found, err)
	}
	if err.Cause != GitHubSetupMissingRealGH {
		t.Fatalf("ResolveRealGH error cause = %v; want GitHubSetupMissingRealGH", err.Cause)
	}
}

// TestEnsureGHWrapper verifies atomic creation and idempotent updating of the wrapper symlink.
func TestEnsureGHWrapper(t *testing.T) {
	tmpDir := t.TempDir()
	selfBin := filepath.Join(tmpDir, "fake-agent-bin")
	if err := os.WriteFile(selfBin, []byte("#!/bin/sh\nexit 0\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	// First creation
	wrapper, err := EnsureGHWrapper(tmpDir, selfBin)
	if err != nil {
		t.Fatalf("EnsureGHWrapper failed: %v", err)
	}
	expectedWrapper := filepath.Join(tmpDir, "run", "gh-trusted", "gh")
	if wrapper != expectedWrapper {
		t.Errorf("EnsureGHWrapper path = %q; want %q", wrapper, expectedWrapper)
	}

	target, linkErr := os.Readlink(wrapper)
	if linkErr != nil {
		t.Fatalf("readlink failed: %v", linkErr)
	}
	if target != selfBin {
		t.Errorf("wrapper target = %q; want %q", target, selfBin)
	}

	// Second creation with same target (idempotent)
	wrapper2, err2 := EnsureGHWrapper(tmpDir, selfBin)
	if err2 != nil || wrapper2 != expectedWrapper {
		t.Fatalf("EnsureGHWrapper idempotent call failed: %v", err2)
	}

	// Update to new target
	newSelfBin := filepath.Join(tmpDir, "fake-agent-v2")
	if err := os.WriteFile(newSelfBin, []byte("#!/bin/sh\nexit 0\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	wrapper3, err3 := EnsureGHWrapper(tmpDir, newSelfBin)
	if err3 != nil || wrapper3 != expectedWrapper {
		t.Fatalf("EnsureGHWrapper update call failed: %v", err3)
	}
	target2, readErr := os.Readlink(wrapper)
	if readErr != nil {
		t.Fatalf("failed to read updated wrapper symlink: %v", readErr)
	}
	if target2 != newSelfBin {
		t.Errorf("updated wrapper target = %q; want %q", target2, newSelfBin)
	}
}

// TestResolveGitHubTrusted verifies resolving trusted accounts with override or git/gh discovery.
func TestResolveGitHubTrusted(t *testing.T) {
	ctx := context.Background()

	// Case 1: explicit override takes precedence without executing subprocesses.
	overrideExec := &fakeGHExecutor{gitErr: errors.New("should not be called"), ghErr: errors.New("should not be called")}
	trusted, err := ResolveGitHubTrusted(ctx, overrideExec, "/dummy", "alice bob")
	if err != nil || trusted != "alice bob" {
		t.Fatalf("ResolveGitHubTrusted with override = (%q, %v); want ('alice bob', nil)", trusted, err)
	}

	// Case 2: repo owner and gh user discovered.
	exec := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "ms-ai-agents",
	}
	trusted, err = ResolveGitHubTrusted(ctx, exec, "/dummy", "")
	if err != nil {
		t.Fatalf("ResolveGitHubTrusted failed: %v", err)
	}
	expected := "mateusz-szczyrzyca ms-ai-agents"
	if trusted != expected {
		t.Errorf("ResolveGitHubTrusted = %q; want %q", trusted, expected)
	}

	// Case 3: repo owner found, gh login fails (graceful fallback to owner only).
	execOwnerOnly := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghErr:     errors.New("not logged in"),
	}
	trusted, err = ResolveGitHubTrusted(ctx, execOwnerOnly, "/dummy", "")
	if err != nil || trusted != "mateusz-szczyrzyca" {
		t.Fatalf("ResolveGitHubTrusted owner only = (%q, %v); want ('mateusz-szczyrzyca', nil)", trusted, err)
	}
}

// TestSetupGitHubRun verifies complete per-run environment assembly and PATH prepending.
func TestSetupGitHubRun(t *testing.T) {
	tmpDir := t.TempDir()
	selfBin := filepath.Join(tmpDir, "agent-bin")
	if err := os.WriteFile(selfBin, []byte("#!/bin/sh\nexit 0\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	exec := &fakeGHExecutor{
		originURL: "https://github.com/mateusz-szczyrzyca/team-agents.git",
		ghUser:    "ms-ai-agents",
	}

	initialEnv := os.Environ()

	params := GitHubSetupParams{
		AgentsHome:       tmpDir,
		SelfExecutable:   selfBin,
		RepoRoot:         tmpDir,
		RunnerID:         "task-123",
		Team:             "team-alpha",
		Agent:            "gemini-1",
		Coordinator:      "codex-1",
		OwnPR:            "42",
		BusDir:           filepath.Join(tmpDir, "bus"),
		AgentbusBin:      filepath.Join(tmpDir, "bin", "agentbus"),
		ReviewByline:     "Review by gemini-1",
		GitHubTrusted:    "",
		GitHubColors:     "auto",
		GitHubNameColors: "gemini-1:cyan",
		CurrentPath:      "/usr/bin:/bin",
		LookPath:         func(_ string) (string, error) { return "/usr/bin/gh", nil }, // Fake successful lookup
	}

	runEnv, err := SetupGitHubRun(context.Background(), exec, params)
	if err != nil {
		t.Fatalf("SetupGitHubRun failed: %v", err)
	}
	if runEnv == nil {
		t.Fatal("SetupGitHubRun returned nil runEnv")
	}

	// Check wrapper dir and PATH prepend
	wrapperDir := filepath.Join(tmpDir, "run", "gh-trusted")
	if runEnv.PrependPath != wrapperDir {
		t.Errorf("PrependPath = %q; want %q", runEnv.PrependPath, wrapperDir)
	}
	expectedPath := wrapperDir + ":/usr/bin:/bin"
	if runEnv.EffectivePath != expectedPath {
		t.Errorf("EffectivePath = %q; want %q", runEnv.EffectivePath, expectedPath)
	}

	// Check exported environment variables
	expectedVars := map[string]string{
		"AGENT_GITHUB_TRUSTED_RESOLVED": "mateusz-szczyrzyca ms-ai-agents",
		"AGENT_GH_COORDINATOR":          "codex-1",
		"AGENT_GH_OWN_PR":               "42",
		"AGENT_GH_TEAM":                 "team-alpha",
		"AGENT_GH_SELF":                 "gemini-1",
		"AGENT_GH_BUS_DIR":              filepath.Join(tmpDir, "bus"),
		"AGENTBUS_BIN":                  filepath.Join(tmpDir, "bin", "agentbus"),
		"AGENT_REVIEW_BYLINE":           "Review by gemini-1",
		"AGENT_GITHUB_COLORS":           "auto",
		"AGENT_GITHUB_NAME_COLORS":      "gemini-1:cyan",
	}

	for k, expectedVal := range expectedVars {
		actualVal, exists := runEnv.Env[k]
		if !exists {
			t.Errorf("runEnv.Env missing key %s", k)
		} else if actualVal != expectedVal {
			t.Errorf("runEnv.Env[%s] = %q; want %q", k, actualVal, expectedVal)
		}
	}

	// Verify no global environment mutation
	currentEnv := os.Environ()
	if len(currentEnv) != len(initialEnv) {
		t.Errorf("global os.Environ length changed from %d to %d", len(initialEnv), len(currentEnv))
	}
}
