package main

import (
	"bytes"
	"context"
	"io"
	"testing"
)

// TestGHDispatchPassThroughStdin verifies that stdin is forwarded to pass-through commands.
func TestGHDispatchPassThroughStdin(t *testing.T) {
	cases := []struct {
		name string
		args []string
	}{
		{name: "generic", args: []string{"secret", "set", "FOO"}},
		{name: "api", args: []string{"api", "graphql"}},
		{name: "diff", args: []string{"pr", "diff", "42"}},
		{name: "plain_view", args: []string{"pr", "view", "42"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var captured []byte
			exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
				for i, a := range cmd.Args {
					if a == "--json" && i+1 < len(cmd.Args) {
						if _, err := cmd.Stdout.Write([]byte(`{"author":{"login":"alice"},"isCrossRepository":false}`)); err != nil {
							return CommandResult{ExitCode: 1}, &ExecuteError{Err: err}
						}
						return CommandResult{ExitCode: 0}, nil
					}
				}
				if cmd.Stdin != nil {
					data, err := io.ReadAll(cmd.Stdin)
					if err != nil {
						return CommandResult{ExitCode: 1}, &ExecuteError{Err: err}
					}
					captured = data
				}
				return CommandResult{ExitCode: 0}, nil
			}}
			input := []byte("hello-stdin-stream")
			code, err := ghDispatch(context.Background(), GHDispatchParams{
				Args:   tc.args,
				Env:    []string{"AGENT_REAL_GH=/usr/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=alice"},
				Stdin:  bytes.NewReader(input),
				Stdout: &bytes.Buffer{},
				Stderr: &bytes.Buffer{},
				Exec:   exec,
			})
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if code != 0 {
				t.Fatalf("unexpected exit code: %d", code)
			}
			if !bytes.Equal(captured, input) {
				t.Fatalf("stdin mismatch: got %q, want %q", string(captured), string(input))
			}
		})
	}
}
