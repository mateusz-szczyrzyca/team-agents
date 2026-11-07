package main

import (
	"bytes"
	"context"
	"io"
	"testing"
)

// TestRound2DispatchAckUsage ensures native acknowledgement validates its target.
func TestRound2DispatchAckUsage(t *testing.T) {
	app := &App{}
	var output bytes.Buffer
	failure := app.dispatch(context.Background(), "ack", nil, &output, io.Discard)
	if failure == nil || failure.Cause != CommandInvalidArgument {
		t.Fatalf("missing acknowledgement target: %v", failure)
	}
	if output.Len() != 0 {
		t.Fatalf("unexpected stdout for invalid acknowledgement: %q", output.String())
	}
}

// TestRound2DispatchJSON uses loaded configuration through the native dispatcher.
func TestRound2DispatchJSON(t *testing.T) {
	app := &App{config: agentsFixtureConfig()}
	var output bytes.Buffer
	if failure := app.dispatch(context.Background(), "_agents-json", nil, &output, io.Discard); failure != nil {
		t.Fatal(failure)
	}
	if records := bytes.Count(output.Bytes(), []byte{'\n'}); records != 7 {
		t.Fatalf("JSON records = %d, want 7: %q", records, output.String())
	}
	if !bytes.Contains(output.Bytes(), []byte(`"model":"quoted\"model"`)) {
		t.Fatalf("loaded configuration missing from JSON: %q", output.String())
	}
}
