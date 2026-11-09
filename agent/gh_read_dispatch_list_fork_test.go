package main

import (
	"bytes"
	"context"
	"encoding/json"
	"strings"
	"testing"
)

// TestGHDispatchListForkFiltering verifies that isCrossRepository is requested
// when pr list --json is called, so fork PRs from trusted authors are filtered.
func TestGHDispatchListForkFiltering(t *testing.T) {
	t.Run("pr_list_json_fork_from_trusted_author_filtered", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{
			run: func(cmd Command) (CommandResult, *ExecuteError) {
				if cmd.Stdout != nil && len(cmd.Args) > 0 && cmd.Args[len(cmd.Args)-1] != "" {
					jsonFieldIdx := -1
					for i, arg := range cmd.Args {
						if arg == "--json" && i+1 < len(cmd.Args) {
							jsonFieldIdx = i + 1
							break
						}
					}
					if jsonFieldIdx > 0 {
						flds := cmd.Args[jsonFieldIdx]
						if !strings.Contains(flds, "isCrossRepository") {
							t.Fatalf("isCrossRepository not in requested fields: %s", flds)
						}
					}
					if n, err := cmd.Stdout.Write([]byte(`[
						{"number":1,"title":"Own PR","author":{"login":"trusted_user"},"isCrossRepository":false},
						{"number":2,"title":"Fork from trusted","author":{"login":"trusted_user"},"isCrossRepository":true}
					]`)); err != nil || n == 0 {
						t.Fatalf("failed to write pr list response: %v", err)
					}
				}
				return CommandResult{ExitCode: 0}, nil
			},
		}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   []string{"pr", "list", "--json", "number,title"},
			Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if code != 0 || err != nil {
			t.Fatalf("code=%d err=%v", code, err)
		}
		out := stdout.String()
		var result []map[string]any
		if err := json.Unmarshal([]byte(out), &result); err != nil {
			t.Fatalf("failed to unmarshal result: %v", err)
		}
		if len(result) != 1 {
			t.Fatalf("expected 1 PR (fork filtered), got %d: %s", len(result), out)
		}
		if num, ok := result[0]["number"].(float64); !ok || num != 1 {
			t.Fatalf("expected PR #1, got %v", result[0]["number"])
		}
	})
}
