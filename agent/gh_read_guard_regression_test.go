package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestGHDispatchWriteGuardBeforeFormatting preserves title flags and PR guard priority.
func TestGHDispatchWriteGuardBeforeFormatting(t *testing.T) {
	for _, sub := range []string{"create", "edit", "merge", "ready", "close", "reopen"} {
		t.Run(sub, func(t *testing.T) {
			var stdout, stderr bytes.Buffer
			home := t.TempDir()
			if err := os.MkdirAll(filepath.Join(home, "agents/logs"), 0o700); err != nil {
				t.Fatal(err)
			}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   []string{"pr", sub, "-t", "Title", "-w"},
				Env:    []string{"HOME=" + home, "AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead"},
				Stdout: &stdout, Stderr: &stderr, Exec: fakeGHExec{},
			})
			if code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused {
				t.Fatalf("code=%d err=%v stderr=%s", code, err, stderr.String())
			}
			raw, readErr := os.ReadFile(filepath.Clean(filepath.Join(home, "agents/logs/agent.jsonl")))
			if readErr != nil || !bytes.Contains(raw, []byte(EventGitHubMemberPRRefused)) {
				t.Fatalf("member refusal log=%s err=%v", raw, readErr)
			}
		})
	}
	t.Run("second_pr", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		executor := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if _, err := io.WriteString(cmd.Stdout, `{"state":"OPEN"}`); err != nil {
				t.Fatal(err)
			}
			return CommandResult{}, nil
		}}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   []string{"pr", "create", "-t", "Title"},
			Env:    []string{"HOME=" + t.TempDir(), "AGENT_REAL_GH=/bin/gh", "AGENT_GH_OWN_PR=42"},
			Stdout: &stdout, Stderr: &stderr, Exec: executor,
		})
		if code != 1 || err == nil || err.Cause != GHReadDispatchSecondPRRefused {
			t.Fatalf("code=%d err=%v", code, err)
		}
	})
}

// TestGHDispatchCoordinatorFailureFields retains bus exit status and bounded stderr in warnings.
func TestGHDispatchCoordinatorFailureFields(t *testing.T) {
	for _, exitCode := range []int{7, 127} {
		t.Run(fmt.Sprint(exitCode), func(t *testing.T) {
			home := t.TempDir()
			if err := os.MkdirAll(filepath.Join(home, "agents/logs"), 0o700); err != nil {
				t.Fatal(err)
			}
			var stdout, stderr bytes.Buffer
			executor := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stderr != nil {
					if _, err := io.WriteString(cmd.Stderr, strings.Repeat("x", 600)); err != nil {
						t.Fatal(err)
					}
				}
				return CommandResult{ExitCode: exitCode}, nil
			}}
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   []string{"pr", "create"},
				Env:    []string{"HOME=" + home, "AGENT_REAL_GH=/bin/gh", "AGENT_GH_TEAM=test-team", "AGENT_GH_COORDINATOR=lead"},
				Stdout: &stdout, Stderr: &stderr, Exec: executor,
			})
			if code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused {
				t.Fatalf("code=%d err=%v", code, err)
			}
			raw, readErr := os.ReadFile(filepath.Clean(filepath.Join(home, "agents/logs/agent.jsonl")))
			if readErr != nil {
				t.Fatal(readErr)
			}
			var event map[string]string
			if decodeErr := json.Unmarshal(bytes.Split(raw, []byte("\n"))[0], &event); decodeErr != nil {
				t.Fatal(decodeErr)
			}
			if event["event"] != string(EventGitHubCoordinatorUnknown) || event["exit_code"] != fmt.Sprint(exitCode) || event["stderr"] != strings.Repeat("x", 500) {
				t.Fatalf("coordinator warning=%v", event)
			}
		})
	}
}
