package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"testing"
)

// TestRound2DispatchAck preserves typed handler failures through native dispatch.
func TestRound2DispatchAck(t *testing.T) {
	for _, tc := range []struct {
		name            string       // name identifies the handler outcome.
		target          string       // target selects a task or ambiguous fragment.
		status          string       // status supplies the fixture's task state.
		markerDirectory bool         // markerDirectory forces marker creation to fail.
		cause           CommandCause // cause is the public command failure, or zero on success.
		nested          AckCause     // nested verifies the original acknowledgement failure survives.
	}{
		{name: "success", target: "task", status: "done"},
		{name: "missing", target: "absent", status: "done", cause: CommandNotFound, nested: AckNotFound},
		{name: "ambiguous", target: "ta", status: "done", cause: CommandConflict, nested: AckAmbiguous},
		{name: "running", target: "task", status: "running", cause: CommandConflict, nested: AckStillRunning},
		{name: "storage", target: "task", status: "done", markerDirectory: true, cause: CommandStorage, nested: AckStorage},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			writeAckFixture(t, root, "task", tc.status)
			writeAckFixture(t, root, "task-two", tc.status)
			if tc.markerDirectory {
				if err := os.Mkdir(filepath.Join(root, "task", "acked"), 0o700); err != nil {
					t.Fatal(err)
				}
			}
			app := &App{tasksDir: root}
			var output bytes.Buffer // output captures the resolved task acknowledgement.
			failure := app.dispatch(context.Background(), "ack", []string{tc.target}, &output, io.Discard)
			if tc.name == "success" {
				if failure != nil || output.String() != "Acknowledged task.\n" {
					t.Fatalf("output=%q failure=%v", output.String(), failure)
				}
				return
			}
			var nested *AckError // nested retains the handler cause behind command classification.
			if failure == nil || failure.Cause != tc.cause || !errors.As(failure, &nested) || nested.Cause != tc.nested {
				t.Fatalf("want command %v wrapping ack %v, got %v", tc.cause, tc.nested, failure)
			}
		})
	}
}
