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

// TestGHDispatchAPIShortOptionParsing verifies that short options like -p, compact -q,
// and clustered -iX/-iF forms are properly parsed before endpoint/mutation classification.
func TestGHDispatchAPIShortOptionParsing(t *testing.T) {
	// 1. Preview option -p requires its value to be consumed before endpoint extraction.
	// Without fix: "nebula" becomes endpoint, "comments" is ignored, untrusted comment returned.
	{
		var stdout, stderr bytes.Buffer
		var calledPath string
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				// Capture what path was passed to gh
				for _, arg := range cmd.Args {
					if arg == "repos/o/r/issues/1/comments" || arg == "nebula" {
						calledPath = arg
					}
				}
				stdout.WriteString(`[{"id":"1","author":{"login":"untrusted"},"body":"UNTRUSTED_BODY"}]`)
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
		// Should filter untrusted comments, not treat "nebula" as endpoint
		if code != 0 || err != nil || stdout.String() != "[]" {
			t.Errorf("preview option -p: code=%d err=%v stdout=%q (want filtered comments)", code, err, stdout.String())
		}
	}

	// 2. Compact -q (query) form must be consumed before reaching endpoint.
	// Without fix: compact "-qcompact_query" bypasses filtering.
	{
		var stdout, stderr bytes.Buffer
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				// Return untrusted body
				stdout.WriteString(`["UNTRUSTED_BODY"]`)
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
		// Should filter the result properly
		if code != 0 || err != nil {
			t.Errorf("compact -q: code=%d err=%v (should handle compact query)", code, err)
		}
	}

	// 3. Clustered -iXPATCH (method as -i cluster) must be recognized as mutation.
	// Without fix: method empty, no mutation check, PATCH executed.
	{
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "repos/o/r/pulls/1", "-iXPATCH"}
		env := []string{"AGENT_REAL_GH=/bin/gh"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		// Should refuse write, not call gh
		if calledRealGH || code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
			t.Errorf("clustered -iXPATCH: called=%v code=%d err=%v (should refuse write)", calledRealGH, code, err)
		}
	}

	// 4. Clustered -iFquery=@file must be recognized as field with opaque input.
	// Without fix: bypasses opaque-query detection, file loaded and POST sent.
	{
		var stdout, stderr bytes.Buffer
		calledRealGH := false
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledRealGH = true
			}
			return CommandResult{ExitCode: 0}, nil
		}}
		args := []string{"api", "graphql", "-iFquery=@payload.graphql"}
		env := []string{"AGENT_REAL_GH=/bin/gh"}
		code, err := ghDispatch(context.Background(), GHDispatchParams{
			Args:   args,
			Env:    env,
			Stdout: &stdout,
			Stderr: &stderr,
			Exec:   exec,
		})
		// Should refuse opaque input, not call gh
		if calledRealGH || code != 1 {
			t.Errorf("clustered -iFquery: called=%v code=%d (should refuse opaque input)", calledRealGH, code)
		}
	}
}
