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

// TestGHDispatchAPICacheAndPassthroughFlags verifies -c cache option is treated as a two-token
// flag without corrupting endpoint path, and --comments/--json pass through intact without stripping.
func TestGHDispatchAPICacheAndPassthroughFlags(t *testing.T) {
	// 1. -c 1h before comments endpoint must preserve arguments and filter comments.
	{
		var stdout, stderr bytes.Buffer
		var capturedArgs []string
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			capturedArgs = cmd.Args
			_, _ = cmd.Stdout.Write([]byte(`[{"id":"1","author":{"login":"untrusted_user"},"body":"hello"}]`))
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "-c", "1h", "repos/o/r/issues/7/comments"}
		env := []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		if code != 0 || err != nil {
			t.Fatalf("code=%d err=%v stderr=%q", code, err, stderr.String())
		}
		if len(capturedArgs) < 4 || capturedArgs[1] != "-c" || capturedArgs[2] != "1h" {
			t.Fatalf("captured args missing -c 1h: %v", capturedArgs)
		}
		if strings.Contains(stdout.String(), "untrusted_user") {
			t.Fatalf("untrusted comment was not filtered: %q", stdout.String())
		}
	}

	// 2. --comments and --json flags must pass through to real gh without being stripped.
	{
		var capturedArgs []string
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			capturedArgs = cmd.Args
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "repos/o/r/issues/7", "--comments", "--json", "body"}
		env := []string{"AGENT_REAL_GH=/bin/gh"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &bytes.Buffer{},
			Stderr: &bytes.Buffer{},
			Exec:   exec,
		})
		if code != 0 || err != nil {
			t.Fatalf("code=%d err=%v", code, err)
		}
		want := "api repos/o/r/issues/7 --comments --json body"
		if strings.Join(capturedArgs, " ") != want {
			t.Fatalf("captured args = %q, want %q", strings.Join(capturedArgs, " "), want)
		}
	}
}

