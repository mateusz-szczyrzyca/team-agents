package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

// TestGHDispatchAPIShortOptions verifies that short options (-p, compact -q,
// and clustered -iX / -iF) are parsed properly before endpoint extraction
// and mutation classification.
func TestGHDispatchAPIShortOptions(t *testing.T) {
	// 1. Preview option -p must consume its value so it does not corrupt the endpoint path.
	t.Run("preview_flag_preserves_endpoint", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		var calledArgs []string
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledArgs = cmd.Args
				if cmd.Stdout != nil {
					if _, err := cmd.Stdout.Write([]byte(`[{"id":"1","author":{"login":"outsider"},"body":"untrusted"}]`)); err != nil {
						t.Fatalf("stdout write failed: %v", err)
					}
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
		if code != 0 || err != nil {
			t.Fatalf("preview option -p failed: code=%d err=%v", code, err)
		}
		if stdout.String() != "[]\n" {
			t.Fatalf("preview option -p did not filter comments: got %q, want []\\n", stdout.String())
		}
		foundPreview, foundEndpoint := false, false
		for idx, arg := range calledArgs {
			if arg == "-p" && idx+1 < len(calledArgs) && calledArgs[idx+1] == "nebula" {
				foundPreview = true
			}
			if arg == "repos/o/r/issues/1/comments" {
				foundEndpoint = true
			}
		}
		if !foundPreview || !foundEndpoint {
			t.Fatalf("preview option -p args not forwarded properly: %v", calledArgs)
		}
	})

	// 2. Compact -q (e.g. -qmap(.body)) must be extracted into jq and stripped from real gh args.
	t.Run("compact_jq_query_stripped_before_filter", func(t *testing.T) {
		var stdout, stderr bytes.Buffer
		var calledArgs []string
		exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
			if cmd.Name == "/bin/gh" {
				calledArgs = cmd.Args
				if cmd.Stdout != nil {
					// Real gh must receive un-queried comments to preserve author metadata.
					if _, err := cmd.Stdout.Write([]byte(`[{"id":"1","author":{"login":"outsider"},"body":"untrusted"}]`)); err != nil {
						t.Fatalf("stdout write failed: %v", err)
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
		if code != 0 || err != nil {
			t.Fatalf("compact -q failed: code=%d err=%v", code, err)
		}
		for _, arg := range calledArgs {
			if strings.HasPrefix(arg, "-q") {
				t.Fatalf("compact -q was forwarded to real gh before filtering: %v", calledArgs)
			}
		}
		if stdout.String() != "[]\n" {
			t.Fatalf("compact -q did not filter outsider comment: got %q, want []\\n", stdout.String())
		}
	})

	// 3. Clustered -iXPATCH must be recognized as HTTP method PATCH and refused as write.
	t.Run("clustered_ix_method_refused", func(t *testing.T) {
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
		if calledRealGH {
			t.Fatalf("clustered -iXPATCH called real gh instead of refusing")
		}
		if code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
			t.Fatalf("clustered -iXPATCH expected WriteUnavailable error: code=%d err=%v", code, err)
		}
	})

	// 4. Clustered -iFquery=@file must be recognized as typed GraphQL query file and refused.
	t.Run("clustered_if_query_file_refused", func(t *testing.T) {
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
		if calledRealGH {
			t.Fatalf("clustered -iFquery called real gh instead of refusing")
		}
		if code != 1 || err == nil || err.Cause != GHReadDispatchWriteUnavailable {
			t.Fatalf("clustered -iFquery expected WriteUnavailable error: code=%d err=%v", code, err)
		}
	})
}
