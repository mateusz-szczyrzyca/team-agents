package main

import (
	"io"
)

// dispatch routes a command to its handler, returning a typed CommandError on failure.
func dispatch(cmd string, args []string, stdout, stderr io.Writer) *CommandError {
	return nil
}

// run executes the agent CLI command with the given arguments, writing output to stdout
// and diagnostics to stderr. It returns the process exit code.
func run(args []string, stdout, _ io.Writer) int {
	cmd := "help"
	if len(args) > 0 {
		cmd = args[0]
	}

	switch cmd {
	case "help", "-h", "--help":
		if err := printUsage(stdout); err != nil {
			return 1
		}
		return 0
	default:
		if err := printUsage(stdout); err != nil {
			return 1
		}
		return 1
	}
}
