package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// realNodeBridgeFixture configures the repository script in a temporary home and never permits a real run.
func realNodeBridgeFixture(t *testing.T) *nodeProcessExecutor {
	t.Helper()
	home := t.TempDir()
	for _, dir := range []string{"node", "tasks/task-one"} {
		if err := os.MkdirAll(filepath.Join(home, dir), 0o700); err != nil {
			t.Fatal(err)
		}
	}
	cfg := "declare -A AGENT_TYPE=([codex-1]=codex)\nNORMAL_AGENTS=(codex-1)\nWORKER_AGENTS=()\nINSTANCE_NAME=mac\nINSTANCE_ACCEPT_REPOS=(different-repository)\nQUESTIONS_EDITOR=off\n"
	if err := os.WriteFile(filepath.Join(home, "agents.conf"), []byte(cfg), 0o600); err != nil {
		t.Fatal(err)
	}
	meta := "ID=task-one\nAGENT=codex-1\nKIND=worker\nTEAM=''\nTIER=normal\nTYPE=codex\nREPO_NAME=example-app\nRUN=1\nROUND=2\nSTATUS=waiting\nCREATED=2026-10-22T16:00:00Z\n"
	if err := os.WriteFile(filepath.Join(home, "tasks", "task-one", "meta.env"), []byte(meta), 0o600); err != nil {
		t.Fatal(err)
	}
	questions := "# Questions\n\n## 1. Example\na) Option\n\n**Recommended:** a)\n\n**Verdict:**\n"
	if err := os.WriteFile(filepath.Join(home, "tasks", "task-one", "questions-2.md"), []byte(questions), 0o600); err != nil {
		t.Fatal(err)
	}
	script, err := filepath.Abs(filepath.Join("..", "bin", "agent"))
	if err != nil {
		t.Fatal(err)
	}
	executor := &nodeProcessExecutor{Home: home, AgentCmd: script, Instance: "mac"}
	t.Cleanup(func() {
		if err := executor.Close(); err != nil {
			t.Error(err)
		}
	})
	return executor
}

// TestNodeRealBridgeManifestAndReplay verifies sourced arrays, actual metadata and the desired-run no-op.
func TestNodeRealBridgeManifestAndReplay(t *testing.T) {
	executor := realNodeBridgeFixture(t)
	current, err := nodeCurrentState(executor.AgentCmd, executor.Home)
	if err != nil {
		t.Fatal(err)
	}
	if len(current.Agents) != 1 || current.Agents[0].Role != string(RoleAgent) || current.Tasks["task-one"].Agent != "codex-1" {
		t.Fatalf("actual bridge manifest: %+v", current)
	}
	result, err := executor.Execute(context.Background(), Command{Kind: CommandStartRun, Task: "task-one", Params: map[string]string{"agent": "codex-1", "run": "1"}}, false)
	if err != nil || result.Outcome != CommandOutcomeAlreadyRunning {
		t.Fatalf("desired-run replay: %+v %v", result, err)
	}
}

// TestNodeRealBridgeRepositoryDenial happens before _runner, so this test cannot launch a real agent CLI.
func TestNodeRealBridgeRepositoryDenial(t *testing.T) {
	executor := realNodeBridgeFixture(t)
	_, err := executor.Execute(context.Background(), Command{Kind: CommandStartRun, Task: "task-one", Params: map[string]string{"agent": "codex-1", "run": "2"}}, false)
	if err == nil {
		t.Fatal("real bridge ignored the local repository policy")
	}
	data, readErr := os.ReadFile(filepath.Join(executor.Home, "tasks", "task-one", "meta.env"))
	if readErr != nil {
		t.Fatal(readErr)
	}
	if !strings.Contains(string(data), "RUN=1\n") {
		t.Fatal("repository denial advanced the run")
	}
}

// TestNodeRealBridgeRejectsStaleAnswers preserves both the questions snapshot and the current run.
func TestNodeRealBridgeRejectsStaleAnswers(t *testing.T) {
	executor := realNodeBridgeFixture(t)
	before, readErr := os.ReadFile(filepath.Join(executor.Home, "tasks", "task-one", "questions-2.md"))
	if readErr != nil {
		t.Fatal(readErr)
	}
	for _, round := range []string{"3", "2"} {
		command := Command{Kind: CommandAnswerDeliver, Task: "task-one", Params: map[string]string{"answer": "## 1. Example\n**Verdict:** ok\n", "question_round": round, "snapshot_version": strings.Repeat("0", 64)}}
		if _, err := executor.Execute(context.Background(), command, false); err == nil {
			t.Fatal("stale answer was delivered")
		}
	}
	after, readErr := os.ReadFile(filepath.Join(executor.Home, "tasks", "task-one", "questions-2.md"))
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(after) != string(before) {
		t.Fatal("stale delivery changed questions")
	}
}
