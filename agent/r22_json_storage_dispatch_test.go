package main

import (
	"context"
	"errors"
	"io"
	"testing"
)

// TestRound2DispatchJSONStorage preserves the nested storage cause and write error.
func TestRound2DispatchJSONStorage(t *testing.T) {
	app := &App{config: agentsFixtureConfig()}
	failure := app.dispatch(context.Background(), "_agents-json", nil, ackFailWriter{}, io.Discard)
	var listingError *AgentsJSONError
	if failure == nil || failure.Cause != CommandStorage || !errors.As(failure, &listingError) || listingError.Cause != AgentsJSONStorage || !errors.Is(failure, io.ErrClosedPipe) {
		t.Fatalf("JSON write failure lost cause: %v", failure)
	}
}
