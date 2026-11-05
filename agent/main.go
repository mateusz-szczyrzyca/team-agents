package main

import (
	"context"
	"os"
)

// main is the entry point for the Go implementation of the agent binary.
func main() {
	app := newApp(OSExecutor{}, nil, nil)
	if err := app.loadStartupConfig(context.Background(), os.Args[1:]); err != nil {
		os.Exit(err.Class().ExitCode())
	}
	os.Exit(app.run(os.Args[1:], os.Stdout, os.Stderr))
}
