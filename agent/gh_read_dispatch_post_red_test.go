package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

// TestGHDispatchWriteOutShortWrite verifies detection of incomplete writes.
func TestGHDispatchWriteOutShortWrite(t *testing.T) {
	err := writeOut(shortWriter{}, "hello")
	if err == nil {
		t.Fatalf("expected error on short write, got nil")
	}
}

// TestGHDispatchViewOptionFlagsPreserved verifies options like --repo are preserved in view/diff/checkout.
func TestGHDispatchViewOptionFlagsPreserved(t *testing.T) {
	var capturedArgs []string
	exec := fakeGHExec{run: func(cmd Command) (CommandResult, *ExecuteError) {
		capturedArgs = cmd.Args
		if cmd.Stdout != nil {
			_, _ = cmd.Stdout.Write([]byte(`{"author":{"login":"trusted_user"},"isCrossRepository":false}`))
		}
		return CommandResult{ExitCode: 0}, nil
	}}
	p := GHDispatchParams{
		Args:   []string{"pr", "view", "10", "--repo", "my-org/my-repo"},
		Env:    []string{"AGENT_REAL_GH=/bin/gh", "AGENT_GITHUB_TRUSTED_RESOLVED=trusted_user"},
		Stdout: &bytes.Buffer{},
		Stderr: &bytes.Buffer{},
		Exec:   exec,
	}
	code, err := ghDispatch(context.Background(), p)
	if code != 0 || err != nil {
		t.Fatalf("code=%d err=%v", code, err)
	}
	joined := strings.Join(capturedArgs, " ")
	if !strings.Contains(joined, "--repo my-org/my-repo") {
		t.Fatalf("expected '--repo my-org/my-repo' in args, got: %q", joined)
	}
}
