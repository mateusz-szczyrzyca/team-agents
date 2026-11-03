package main

import (
	"os"
)

// main is the entry point for the Go implementation of the agent binary.
func main() {
	app := newApp(OSExecutor{}, nil, nil)
	os.Exit(app.run(os.Args[1:], os.Stdout, os.Stderr))
}
