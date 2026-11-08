package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestGHDispatchExecutableBoundary verifies that short-option normalization and
// mutation refusals hold at the true executable boundary with OSExecutor.
func TestGHDispatchExecutableBoundary(t *testing.T) {
	tmp := t.TempDir()
	logFile := filepath.Join(tmp, "gh-args.log")
	realGH := filepath.Join(tmp, "fake-gh")
	script := "#!/bin/sh\nprintf '%s\\n' \"$*\" >> " + logFile + "\n" +
		"if [ \"$1\" = \"api\" ]; then\n" +
		"  printf '[{\"id\":\"1\",\"author\":{\"login\":\"outsider\"},\"body\":\"secret\"}]\\n'\n" +
		"fi\nexit 0\n"
	if err := os.WriteFile(realGH, []byte(script), 0o700); err != nil { // #nosec G302 -- test fixture must execute
		t.Fatalf("failed to write fake gh: %v", err)
	}

	env := []string{
		"AGENT_REAL_GH=" + realGH,
		"AGENT_GITHUB_TRUSTED_RESOLVED=alice",
		"PATH=" + os.Getenv("PATH"),
	}

	// 1. Compact -q must be stripped from real gh arguments and evaluated via real jq.
	t.Run("compact_query_boundary", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		args := []string{"api", "repos/o/r/issues/1/comments", "-qmap(.body)"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   OSExecutor{},
		})
		if code != 0 || err != nil {
			t.Fatalf("ghDispatch failed: code=%d err=%v stderr=%s", code, err, stderr.String())
		}
		if stdout.String() != "[]\n" {
			t.Fatalf("filtered output = %q, want []\\n", stdout.String())
		}
		logged, logErr := os.ReadFile(logFile)
		if logErr != nil {
			t.Fatalf("failed to read arg log: %v", logErr)
		}
		if strings.Contains(string(logged), "-q") {
			t.Fatalf("real gh received un-stripped query flag: %s", string(logged))
		}
	})

	// 2. Clustered -iXPATCH must be refused before launching real gh.
	t.Run("clustered_ix_refusal_boundary", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		args := []string{"api", "repos/o/r/pulls/1", "-iXPATCH"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   OSExecutor{},
		})
		if code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
			t.Fatalf("expected WriteUnavailable: code=%d err=%v", code, err)
		}
	})

	// 3. Clustered -iFquery=@file must be refused before launching real gh.
	t.Run("clustered_if_refusal_boundary", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		args := []string{"api", "graphql", "-iFquery=@payload.graphql"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   OSExecutor{},
		})
		if code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
			t.Fatalf("expected WriteUnavailable: code=%d err=%v", code, err)
		}
	})
}
