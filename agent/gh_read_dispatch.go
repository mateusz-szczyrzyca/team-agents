package main

import (
	"context"
	"io"
)

// GHDispatchParams supplies invocation parameters and execution dependencies for ghDispatch.
type GHDispatchParams struct {
	// Args: command arguments (e.g. os.Args[1:]).
	Args []string
	// Env: process environment variables.
	Env []string
	// Stdout: standard output destination.
	Stdout io.Writer
	// Stderr: standard error destination.
	Stderr io.Writer
	// Exec: command executor.
	Exec Executor
}

// ghDispatch evaluates native GitHub read commands and member guards.
func ghDispatch(_ context.Context, _ GHDispatchParams) (int, *GHReadDispatchError) {
	return 0, nil
}
