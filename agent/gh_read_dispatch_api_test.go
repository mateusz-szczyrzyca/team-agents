package main

import (
	"bytes"
	"context"
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
