package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

// nodeExecutorFixture provides fixed synthetic bridges and a FIFO without starting real agent CLIs.
func nodeExecutorFixture(t *testing.T) *nodeProcessExecutor {
	t.Helper()
	home := t.TempDir()
	for _, dir := range []string{"node", "tasks/task-one"} {
		if err := os.MkdirAll(filepath.Join(home, dir), 0o700); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(home, "tasks", "task-one", "meta.env"), []byte("AGENT=codex-1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := syscall.Mkfifo(filepath.Join(home, "control.fifo"), 0o600); err != nil {
		t.Fatal(err)
	}
	current := InstanceCurrentState{Slots: SlotCapacity{Total: 1, Free: 1}, Tasks: map[string]HubTaskSummary{"task-one": {TaskID: "task-one", Agent: "codex-1"}}}
	data, err := json.Marshal(current)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, "report.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(home, "agent-stub")
	content := `#!/bin/sh
set -eu
case "$1" in
_node-manifest) cat "$AGENTS_HOME/report.json" ;;
_node-start|_node-wake) printf 'started\n' >&3; printf 'started\n' > "$AGENTS_HOME/started"; exec cat "$AGENTS_HOME/control.fifo" ;;
_node-answer) cat > "$AGENTS_HOME/answer"; printf 'ready\n' > "$AGENTS_HOME/answer-ready"; printf 'delivered\n' >&3; exec cat "$AGENTS_HOME/control.fifo" ;;
_node-mark-stopped) printf 'stopped\n' > "$AGENTS_HOME/stopped" ;;
*) exit 20 ;;
esac
`
	if err := os.WriteFile(script, []byte(content), 0o700); err != nil { //nolint:gosec // G306: private test stub must be executable; it is never installed outside t.TempDir()
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

// waitNodeFixtureFile waits for deterministic stub evidence with an overall timeout rather than a bare sleep.
func waitNodeFixtureFile(t *testing.T, path string) {
	t.Helper()
	timer := time.NewTimer(3 * time.Second)
	defer timer.Stop()
	ticker := time.NewTicker(time.Millisecond)
	defer ticker.Stop()
	for {
		_, err := os.ReadFile(path) //nolint:gosec // G304: callers select fixed evidence files under the fixture t.TempDir()
		if err == nil {
			return
		}
		if !os.IsNotExist(err) {
			t.Fatal(err)
		}
		select {
		case <-timer.C:
			t.Fatal("stub did not produce expected evidence")
		case <-ticker.C:
		}
	}
}

// TestNodeNativeExecutorStartsOnceAndStopsOwnedGroup verifies real child supervision using only a local stub.
func TestNodeNativeExecutorStartsOnceAndStopsOwnedGroup(t *testing.T) {
	executor := nodeExecutorFixture(t)
	command := Command{Seq: 1, Kind: CommandStartRun, Task: "task-one", Params: map[string]string{"agent": "codex-1", "run": "1"}}
	result, err := executor.Execute(context.Background(), command, false)
	if err != nil || result.Outcome != CommandOutcomeStarted {
		t.Fatalf("start: %+v %v", result, err)
	}
	waitNodeFixtureFile(t, filepath.Join(executor.Home, "started"))
	result, err = executor.Execute(context.Background(), command, false)
	if err != nil || result.Outcome != CommandOutcomeAlreadyRunning {
		t.Fatalf("duplicate: %+v %v", result, err)
	}
	result, err = executor.Execute(context.Background(), Command{Seq: 2, Kind: CommandStopTask, Task: "task-one"}, false)
	if err != nil || result.Outcome != CommandOutcomeStopped {
		t.Fatalf("stop: %+v %v", result, err)
	}
	waitNodeFixtureFile(t, filepath.Join(executor.Home, "stopped"))
	executor.mu.Lock()
	remaining := len(executor.children)
	executor.mu.Unlock()
	if remaining != 0 {
		t.Fatal("owned child remained after stop")
	}
}

// TestNodeNativeExecutorAdmission rejects unknown tasks, traversal, wrong agents and unapproved sensitive work.
func TestNodeNativeExecutorAdmission(t *testing.T) {
	executor := nodeExecutorFixture(t)
	command := Command{Kind: CommandStartRun, Task: "missing", Params: map[string]string{"agent": "codex-1", "run": "1"}}
	result, err := executor.Execute(context.Background(), command, false)
	if err != nil || result.Outcome != CommandOutcomeUnknownTask {
		t.Fatalf("unknown: %+v %v", result, err)
	}
	command.Task = "../escape"
	if _, err := executor.Execute(context.Background(), command, false); err == nil {
		t.Fatal("traversal accepted")
	}
	command.Task = "task-one"
	command.Params["agent"] = "someone-else"
	if _, err := executor.Execute(context.Background(), command, false); err == nil {
		t.Fatal("wrong task agent accepted")
	}
	command.Params["agent"] = "codex-1"
	command.Params["requires"] = "sign:apple"
	executor.Capabilities = []Capability{"sign:apple"}
	if _, err := executor.Execute(context.Background(), command, false); err == nil {
		t.Fatal("sensitive capability ran without approval")
	}
	if err := os.WriteFile(filepath.Join(executor.Home, "node", "approved-capabilities.json"), []byte(`["sign:apple"]`), 0o600); err != nil {
		t.Fatal(err)
	}
	result, err = executor.Execute(context.Background(), command, false)
	if err != nil || result.Outcome != CommandOutcomeStarted {
		t.Fatalf("approved start: %+v %v", result, err)
	}
}

// TestNodeNativeExecutorDoesNotRespawnUnknownCrashOutcome keeps ambiguous process launches explicit.
func TestNodeNativeExecutorDoesNotRespawnUnknownCrashOutcome(t *testing.T) {
	executor := nodeExecutorFixture(t)
	result, err := executor.Execute(context.Background(), Command{Kind: CommandStartRun, Task: "task-one"}, true)
	if err != nil || result.Outcome != CommandOutcomeFailed || result.Detail == "" {
		t.Fatalf("unknown outcome: %+v %v", result, err)
	}
	if _, err := os.Stat(filepath.Join(executor.Home, "started")); !os.IsNotExist(err) {
		t.Fatalf("uncertain start caused a side effect: %v", err)
	}
}

// TestNodeNativeExecutorAnswersUseStdin verifies answer text is delivered through the fixed bridge.
func TestNodeNativeExecutorAnswersUseStdin(t *testing.T) {
	executor := nodeExecutorFixture(t)
	command := Command{Kind: CommandAnswerDeliver, Task: "task-one", Params: map[string]string{"answer": "## 1. example\n**Verdict:** ok\n", "question_round": "1", "snapshot_version": strings.Repeat("0", 64)}}
	result, err := executor.Execute(context.Background(), command, false)
	if err != nil || result.Outcome != CommandOutcomeDelivered {
		t.Fatalf("answer: %+v %v", result, err)
	}
	waitNodeFixtureFile(t, filepath.Join(executor.Home, "answer-ready"))
	data, readErr := os.ReadFile(filepath.Join(executor.Home, "answer"))
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(data) != command.Params["answer"] {
		t.Fatalf("answer changed: %q", data)
	}
}

// TestNodeNativeExecutorReservesOneTaskPerAgent rejects a second task even when the hub routes it to the same model.
func TestNodeNativeExecutorReservesOneTaskPerAgent(t *testing.T) {
	executor := nodeExecutorFixture(t)
	current := InstanceCurrentState{Slots: SlotCapacity{Total: 2, Free: 1}, Tasks: map[string]HubTaskSummary{
		"task-one": {TaskID: "task-one", Agent: "codex-1"},
		"task-two": {TaskID: "task-two", Agent: "codex-1", Status: nodeTaskRunning},
	}}
	data, err := json.Marshal(current)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(executor.Home, "report.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
	command := Command{Kind: CommandStartRun, Task: "task-one", Params: map[string]string{"agent": "codex-1", "run": "1"}}
	if _, err := executor.Execute(context.Background(), command, false); err == nil {
		t.Fatal("configured agent was assigned to two local tasks")
	}
}

// TestNodeUnknownOutcomePrecedesChangedPolicy preserves uncertainty even if approvals change after a crash.
func TestNodeUnknownOutcomePrecedesChangedPolicy(t *testing.T) {
	executor := nodeExecutorFixture(t)
	command := Command{Kind: CommandStartRun, Task: "task-one", Params: map[string]string{"requires": "sign:apple"}}
	result, err := executor.Execute(context.Background(), command, true)
	if err != nil || result.Outcome != CommandOutcomeFailed {
		t.Fatalf("uncertain execution was incorrectly classified as a fresh policy denial: %+v %v", result, err)
	}
}
