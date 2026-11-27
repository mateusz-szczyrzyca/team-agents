package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// invAddFixture runs the real CLI with isolated state and returns stdout, its exit error and the home.
func invAddFixture(t *testing.T, args ...string) (string, error, string) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	home := t.TempDir()
	bin := filepath.Join(home, "agentbus")
	build := exec.CommandContext(ctx, "go", "build", "-o", bin, ".") // #nosec G204 -- fixed compiler builds only this package into a test directory.
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
	flags := []string{"inv-add", "--dir", filepath.Join(home, "bus"), "--tasks-dir", filepath.Join(home, "tasks"), "--from-agent", "lead", "--from-task", "task-lead", "--repo", "example", "--branch", "main", "--question", "Resolve the conflict", "--pr-url", "https://example.com/pull/42", "--pr-number", "42"}
	flags = append(flags, args...)
	cmd := exec.CommandContext(ctx, bin, flags...) // #nosec G204 -- executable and arguments are owned by this isolated fixture.
	cmd.Env = append(os.Environ(), "AGENTS_HOME="+home)
	out, err := cmd.Output()
	return string(out), err, home
}

// invAddFailsWith checks the CLI's exit class and structured cause, plus an unchanged queue.
func invAddFailsWith(t *testing.T, want CmdInvAddCause, args ...string) {
	t.Helper()
	out, err, home := invAddFixture(t, args...)
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != want.info().class.ExitCode() {
		t.Fatalf("exit %v, output %q; want %v", err, out, want.info().class)
	}
	logs, readErr := os.ReadFile(filepath.Join(home, "logs", "agentbus.jsonl"))
	if readErr != nil {
		t.Fatal(readErr)
	}
	found := false
	for _, line := range strings.Split(strings.TrimSpace(string(logs)), "\n") {
		var record struct {
			Event string `json:"event"`
			Cause string `json:"cause"`
		}
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Fatal(err)
		}
		if record.Event == string(LogCommandFailed) {
			found = true
			if record.Cause != want.info().name {
				t.Fatalf("cause %q; want %q", record.Cause, want.info().name)
			}
		}
	}
	if !found {
		t.Fatal("missing command_failed record")
	}
	b := &Bus{dir: filepath.Join(home, "bus")}
	mustWith(t, b, func(st *State) {
		if len(st.Investigations) != 0 || st.NextInvID != 0 {
			t.Fatalf("rejection changed queue: %+v", st.Investigations)
		}
	})
}

func TestInvAddConflictStoresTheItem(t *testing.T) {
	out, err, home := invAddFixture(t, "--kind", "conflict", "--item", "2.3")
	if err != nil {
		t.Fatalf("inv-add: %v", err)
	}
	if out != "1\n" {
		t.Fatalf("stdout %q; want new id on one line", out)
	}
	id, err := strconv.ParseInt(strings.TrimSpace(out), 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	b := &Bus{dir: filepath.Join(home, "bus")}
	mustWith(t, b, func(st *State) {
		inv := st.Investigations[invKey(id)]
		if inv == nil {
			t.Fatal("request missing")
		}
		if inv.Item != "2.3" || inv.Kind != KindConflict || inv.FromAgent != "lead" || inv.FromTask != "task-lead" || inv.Repo != "example" || inv.Branch != "main" || inv.Question != "Resolve the conflict" || inv.PRURL != "https://example.com/pull/42" || inv.PRNumber != 42 {
			t.Fatalf("stored request: %+v", inv)
		}
	})
}

func TestInvAddConflictWithoutItemFails(t *testing.T) {
	invAddFailsWith(t, CmdInvAddMissingItem, "--kind", "conflict")
}
func TestInvAddConflictWithInvalidItemFails(t *testing.T) {
	for _, item := range []string{"0.1", "1.0", "01.2", "1", "1.2.3", "65536.1", "1.65536", "../2.3", "壱.2"} {
		t.Run(item, func(t *testing.T) { invAddFailsWith(t, CmdInvAddInvalidItem, "--kind", "conflict", "--item", item) })
	}
}
func TestInvAddItemWithReviewKindFails(t *testing.T) {
	invAddFailsWith(t, CmdInvAddInvalidItem, "--kind", "review", "--item", "2.3")
}
func TestInvAddSplitIsStillRefused(t *testing.T) {
	invAddFailsWith(t, CmdInvAddInvalidKind, "--kind", "split", "--item", "2.3")
}
