// package main is the agent CLI implementation, a rewrite of bin/agent in Go.
package main

import (
	"fmt"
	"os"
)

// main is the entry point for the agent command.
func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "agent: %v\n", err)
		os.Exit(2)
	}
}

// run executes the agent command with the given arguments and returns a RunError.
func run(argv []string) *RunError {
	return dispatchCommand(argv)
}
