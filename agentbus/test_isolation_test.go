package main

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// TestSuiteDefaultsUseTemporaryHome catches regressions that reintroduce production defaults or logging.
func TestSuiteDefaultsUseTemporaryHome(t *testing.T) {
	if os.Getenv("HOME") != suiteIsolation.root || agentsHome() != filepath.Join(suiteIsolation.root, "agents") || defaultDir() != filepath.Join(agentsHome(), "bus") {
		t.Fatal("test suite defaults escaped the temporary home")
	}
	if suiteIsolation.log == nil || suiteIsolation.log.Name() != filepath.Join(agentsHome(), "logs", "agentbus.jsonl") {
		t.Fatal("test suite logs escaped the temporary home")
	}
}

// TestLegacyLeadershipRecordsWakeWithoutSpawning reproduces the fixture that used to invoke the installed agent.
func TestLegacyLeadershipRecordsWakeWithoutSpawning(t *testing.T) {
	b := installState(t, legacyState)
	suiteIsolation.mu.Lock()
	before := len(suiteIsolation.wakes)
	suiteIsolation.mu.Unlock()
	lead := server(b, "claude-1", func(s *Server) { s.team = "20261015-101500-team" })
	chk(t).ok(toolResult(lead.claimLead()))
	suiteIsolation.mu.Lock()
	defer suiteIsolation.mu.Unlock()
	found := false
	for _, task := range suiteIsolation.wakes[before:] {
		if task == "20261015-101500-team-codex-1" {
			found = true
		}
	}
	if !found {
		t.Fatal("legacy sleeping member's wake was not intercepted")
	}
}

// TestHomeDescriptorGuardRejectsLeakedFiles verifies the real-home check entirely with a synthetic home.
func TestHomeDescriptorGuardRejectsLeakedFiles(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("descriptor audit uses Linux /proc; portable environment isolation is checked separately")
	}
	inherited, err := testOpenFiles()
	if err != nil {
		t.Fatal(err)
	}
	home := t.TempDir()
	path := filepath.Join(home, "agentbus.jsonl")
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY, 0o600) //nolint:gosec // G304: synthetic forbidden-home file inside t.TempDir(), never the real home
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := file.Close(); err != nil {
			t.Error(err)
		}
	})
	leaked, err := testHomeDescriptors([]string{home}, inherited)
	if err != nil || len(leaked) != 1 || leaked[0] != path {
		t.Fatalf("real-home descriptor guard missed a log handle: %v %v", leaked, err)
	}
	leaked, err = testHomeDescriptors([]string{filepath.Join(home, "other")}, inherited)
	if err != nil || len(leaked) != 0 {
		t.Fatalf("guard rejected an unrelated temporary log: %v %v", leaked, err)
	}
}
