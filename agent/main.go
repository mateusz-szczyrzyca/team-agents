package main

import (
	"context"
	"fmt"
	"os"
)

// main is the entry point for the Go implementation of the agent binary.
func main() {
	if err := ghFastPath(os.Args[0]); err != nil {
		if _, writeErr := fmt.Fprintf(os.Stderr, "agent: %s\n", err); writeErr != nil {
			os.Exit(1)
		}
		os.Exit(err.Class().ExitCode())
	}
	app := newApp(OSExecutor{}, nil, nil)
	app.Stdout, app.Stderr = os.Stdout, os.Stderr
	if err := app.loadStartupConfig(context.Background(), os.Args[1:]); err != nil {
		if _, writeErr := fmt.Fprintf(os.Stderr, "agent: %s\n", err); writeErr != nil {
			os.Exit(1)
		}
		os.Exit(err.Class().ExitCode())
	}
	os.Exit(app.run(os.Args[1:], os.Stdout, os.Stderr))
}
