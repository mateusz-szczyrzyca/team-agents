package main

import (
	"bytes"
	"context"
	"io"
	"slices"
	"strings"
	"testing"
)

// TestGHDispatchCLIWriteRoutes verifies CLI activation without activating API mutations.
func TestGHDispatchCLIWriteRoutes(t *testing.T) {
	for _, kind := range []string{"pr", "issue"} {
		for _, verb := range []string{"create", "edit", "comment", "merge", "ready", "close", "reopen", "review"} {
			t.Run(kind+"_"+verb, func(t *testing.T) {
				// body exercises publication cleanup while control verbs preserve their arguments.
				body := "Generated with Claude Code\n\nKeep\n\n"
				// args retains the literal CLI payload supplied by the caller.
				args := []string{kind, verb, "--body", body}
				// want is the command passed to real gh, including shell newline semantics.
				want := slices.Clone(args)
				if verb == "create" || verb == "edit" || verb == "comment" {
					want[3] = "Keep"
				}
				// stdout and stderr capture the forwarded command streams.
				var stdout, stderr bytes.Buffer
				// calls ensures exactly one write reaches the real executable.
				calls := 0
				// executor observes the published arguments and input without any GitHub access.
				executor := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
					calls++
					if cmd.Name != "/bin/gh" || !slices.Equal(cmd.Args, want) {
						t.Fatalf("command=%s %q, want /bin/gh %q", cmd.Name, cmd.Args, want)
					}
					// input verifies that argument transformations leave unrelated stdin available.
					input, err := io.ReadAll(cmd.Stdin)
					if err != nil || string(input) != "native stdin" {
						t.Fatalf("stdin=%q err=%v", input, err)
					}
					if err := writeOut(cmd.Stdout, "write result\n"); err != nil {
						t.Fatal(err)
					}
					return CommandResult{ExitCode: 0}, nil
				}}
				// code and failure describe the complete dispatcher result, not just the body helper.
				code, failure := ghDispatch(context.Background(), GHDispatchParams{
					Args: args, Env: []string{"AGENT_REAL_GH=/bin/gh"},
					Stdin: strings.NewReader("native stdin"), Stdout: &stdout, Stderr: &stderr, Exec: executor,
				})
				if code != 0 || failure != nil || calls != 1 || stdout.String() != "write result\n" || stderr.Len() != 0 {
					t.Fatalf("code=%d err=%v calls=%d stdout=%q stderr=%q", code, failure, calls, stdout.String(), stderr.String())
				}
			})
		}
	}
}

// TestGHDispatchCLIWriteFailure preserves the real command's nonzero exit status.
func TestGHDispatchCLIWriteFailure(t *testing.T) {
	// stdout and stderr receive the real command streams.
	var stdout, stderr bytes.Buffer
	// executor returns a refused GitHub write without invoking an external process.
	executor := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		if cmd.Name != "/bin/gh" || !slices.Equal(cmd.Args, []string{"issue", "close", "42"}) {
			t.Fatalf("unexpected command: %s %q", cmd.Name, cmd.Args)
		}
		if err := writeOut(cmd.Stderr, "write rejected\n"); err != nil {
			t.Fatal(err)
		}
		return CommandResult{ExitCode: 17}, nil
	}}
	// code and failure must retain the executor's status and typed failure cause.
	code, failure := ghDispatch(context.Background(), GHDispatchParams{
		Args: []string{"issue", "close", "42"}, Env: []string{"AGENT_REAL_GH=/bin/gh"},
		Stdout: &stdout, Stderr: &stderr, Exec: executor,
	})
	if code != 17 || failure == nil || failure.Cause != GHReadDispatchExecFailed || stderr.String() != "write rejected\n" {
		t.Fatalf("code=%d err=%v stderr=%q", code, failure, stderr.String())
	}
}
