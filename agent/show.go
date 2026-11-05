package main

import (
	"context"
	"io"
)

// cmdShow displays full details of a single task: metadata, task.md, questions, result, report.
func cmdShow(ctx context.Context, app *App, taskID string, out io.Writer) *ResolveTaskError {
	return nil
}
