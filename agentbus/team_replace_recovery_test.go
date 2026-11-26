package main

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

func TestTeamReplaceInterruptedWritesRecoverOnNextLoad(t *testing.T) {
	b := newBus(t)
	replacementFixture(t, b)
	replacementMessage(t, b, "T", "M", "unread before interruption")
	before := replacementState(t, b)
	state, err := os.ReadFile(b.path("state.json"))
	if err != nil {
		t.Fatal(err)
	}
	messages, err := os.ReadFile(b.path("messages.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := teamReplace(b, "T", "M", "N"); err != nil {
		t.Fatal(err)
	}
	// Reproduce a process interrupted after writing both files, before committing its undo directory.
	dir := b.path("replacement-undo")
	if err := os.Mkdir(dir, 0700); err != nil {
		t.Fatal(err)
	}
	for name, data := range map[string][]byte{"state.json": state, "messages.jsonl": messages, "ready": nil} {
		if err := os.WriteFile(filepath.Join(dir, name), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	if after := replacementState(t, b); !reflect.DeepEqual(before, after) {
		t.Fatal("interrupted replacement did not restore original state")
	}
	restored, err := os.ReadFile(b.path("messages.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if string(restored) != string(messages) {
		t.Fatal("interrupted replacement did not restore original messages")
	}
	if _, err := teamReplace(b, "T", "M", "N"); err != nil {
		t.Fatalf("replacement after recovery: %v", err)
	}
}

func TestTeamReplaceCLIMissingOrEmptyToIsInvalidAndLeavesStateUnchanged(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	bin := filepath.Join(t.TempDir(), "agentbus")
	build := exec.CommandContext(ctx, "go", "build", "-o", bin, ".") // #nosec G204 -- fixed Go command builds this package into the test-owned directory.
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
	for _, extra := range [][]string{nil, {"--to", ""}} {
		b := newBus(t)
		replacementFixture(t, b)
		before := replacementState(t, b)
		args := append([]string{"team-replace", "--dir", b.dir, "--team", "T", "--from", "M"}, extra...)
		cmd := exec.CommandContext(ctx, bin, args...) // #nosec G204 -- binary and arguments belong to this test fixture.
		cmd.Env = append(os.Environ(), "AGENTS_HOME="+t.TempDir())
		out, err := cmd.CombinedOutput()
		var exit *exec.ExitError
		if !errors.As(err, &exit) || exit.ExitCode() != ClassInvalidArgument.ExitCode() {
			t.Fatalf("exit = %v, output %q", err, out)
		}
		if after := replacementState(t, b); !reflect.DeepEqual(before, after) {
			t.Fatal("missing replacement name changed state")
		}
	}
}
