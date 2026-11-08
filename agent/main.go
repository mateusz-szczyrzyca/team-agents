package main

import (
	"context"
	"fmt"
	"os"
)

// main is the entry point for the Go implementation of the agent binary.
func main() {
	if ghFastPath(os.Args[0]) {
		os.Exit(runGH(os.Args[1:], os.Environ(), os.Stdout, os.Stderr))
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
