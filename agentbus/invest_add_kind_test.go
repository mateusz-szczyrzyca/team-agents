package main

import (
	"context"
	"errors"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

func TestInvAddRejectsStructuredKindsBeforeQueueing(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	bin := filepath.Join(t.TempDir(), "agentbus")
	build := exec.CommandContext(ctx, "go", "build", "-o", bin, ".") // #nosec G204 -- fixed Go command builds this package into a test-owned directory.
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
	for _, kind := range []string{"split", "conflict"} {
		t.Run(kind, func(t *testing.T) {
			b := newBus(t)
			tasksDir := t.TempDir()
			cmd := exec.CommandContext(ctx, bin, "inv-add", "--dir", b.dir, "--tasks-dir", tasksDir,
				"--from-agent", "lead", "--from-task", "task-lead", "--repo", "example", "--branch", "main",
				"--question", "Help with the item", "--kind", kind) // #nosec G204 -- binary and arguments belong to this isolated CLI fixture.
			out, err := cmd.CombinedOutput()
			var exit *exec.ExitError // invalid structured kinds must fail before mutating the queue
			if !errors.As(err, &exit) || exit.ExitCode() != ClassInvalidArgument.ExitCode() {
				t.Fatalf("inv-add %s: exit = %v, output %q; want invalid argument", kind, err, out)
			}
			next := exec.CommandContext(ctx, bin, "inv-next", "--dir", b.dir) // #nosec G204 -- test-owned binary and temporary bus directory.
			if out, err := next.CombinedOutput(); err != nil || len(out) != 0 {
				t.Fatalf("inv-next after rejection: error %v, output %q; want empty queue", err, out)
			}
			mustWith(t, b, func(st *State) {
				if len(st.Investigations) != 0 || st.NextInvID != 0 {
					t.Fatalf("rejected request changed queue: %+v", st.Investigations)
				}
			})
		})
	}
}
