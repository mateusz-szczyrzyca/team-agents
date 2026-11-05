package main

import (
	"context"
	"io"
)

// cmdBus displays agentbus status and the latest log entries.
func (a *App) cmdBus(_ context.Context, _ string, _ io.Writer) *CommandError {
	return nil
}

// cmdHelpers lists active helper agents.
func (a *App) cmdHelpers(_ context.Context, _ []string, _ io.Writer) *CommandError {
	return nil
}

// cmdInv displays the investigation queue or details of an investigation.
func (a *App) cmdInv(_ context.Context, _ []string, _ io.Writer) *CommandError {
	return nil
}

// cmdNotes displays or searches notebook entries.
func (a *App) cmdNotes(_ context.Context, _ []string, _ io.Writer) *CommandError {
	return nil
}
