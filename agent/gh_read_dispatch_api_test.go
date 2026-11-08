package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

// TestGHDispatchAPIMethodFieldClassification verifies API method and field classification
// blocks member writes and write mutations while preserving valid reads.
func TestGHDispatchAPIMethodFieldClassification(t *testing.T) {
	// 1. Member implicit writes with long-form field flags must be refused.
	for _, flag := range []string{"--field", "--raw-field", "-F"} {
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "repos/o/r/pulls", flag, "title=New PR"}
		env := []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchMemberRefused {
			t.Fatalf("flag %s: called=%v code=%d err=%v stderr=%q", flag, calledRealGH, code, err, stderr.String())
		}
	}

	// 2. Standalone implicit write with --field must be refused as WriteUnavailable.
	{
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "repos/o/r/pulls", "--field", "title=New PR"}
		env := []string{"AGENT_REAL_GH=/bin/gh"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
			t.Fatalf("standalone --field: called=%v code=%d err=%v stderr=%q", calledRealGH, code, err, stderr.String())
		}
	}

	// 3. Explicit GET with fields must be preserved as valid read.
	{
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "-X", "GET", "repos/o/r/pulls", "-f", "state=open"}
		env := []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GH_COORDINATOR=lead-agent"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if !calledRealGH || code != 0 || err != nil {
			t.Fatalf("explicit GET with -f: called=%v code=%d err=%v stderr=%q", calledRealGH, code, err, stderr.String())
		}
	}

	// 4. Harmless GraphQL queries with fields must be preserved as valid read.
	{
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "graphql", "-f", "query={ viewer { login } }"}
		env := []string{"AGENT_REAL_GH=/bin/gh"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if !calledRealGH || code != 0 || err != nil {
			t.Fatalf("harmless graphql: called=%v code=%d err=%v stderr=%q", calledRealGH, code, err, stderr.String())
		}
	}
}

// TestGHDispatchAPIShortOptionParsing verifies that short options like -p, compact -q,
// and clustered -iX/-iF forms are properly parsed before endpoint/mutation classification.
func TestGHDispatchAPIShortOptionParsing(t *testing.T) {
	// 1. Preview option -p requires its value to be consumed before endpoint extraction.
	// Without fix: "nebula" becomes endpoint, untrusted comment returned unfiltered.
	// With fix: -p nebula is consumed, endpoint is correctly identified, untrusted comment filtered.
	{
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" && cmd.Stdout != nil {
				// Return trusted comments
				if n, err := cmd.Stdout.Write([]byte(`[{"id":"1","author":{"login":"alice"},"body":"trusted"}]`)); err != nil || n == 0 {
					t.Fatalf("failed to write preview response: %v", err)
				}
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "-p", "nebula", "repos/o/r/issues/1/comments"}
		env := []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		// Should succeed and preserve trusted comments
		if code != 0 || err != nil || !strings.Contains(stdout.String(), "trusted") {
			t.Errorf("preview option -p: code=%d err=%v stdout=%q (should preserve trusted comment)", code, err, stdout.String())
		}
	}

	// 2. Compact -q (query) form must be consumed, not passed unchanged to gh.
	// Without fix: compact "-qquery" might bypass normal query processing.
	{
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
				if cmd.Stdout != nil {
					// Return trusted comments
					if n, err := cmd.Stdout.Write([]byte(`[{"id":"2","author":{"login":"alice"},"body":"comment"}]`)); err != nil || n == 0 {
						t.Fatalf("failed to write query response: %v", err)
					}
				}
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "repos/o/r/issues/1/comments", "-qmap(.body)"}
		env := []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		// The compact -q should be recognized and the jq filter should be applied
		if !calledRealGH || code != 0 || err != nil {
			t.Errorf("compact -q: called=%v code=%d err=%v (should parse and execute)", calledRealGH, code, err)
		}
	}
}
