package main

import (
	"io"
)

// run executes the agent CLI command with the given arguments, writing output to stdout
// and diagnostics to stderr. It returns the process exit code.
func run(args []string, stdout, stderr io.Writer) int {
	cmd := "help"
	if len(args) > 0 {
		cmd = args[0]
	}

	switch cmd {
	case "help", "-h", "--help":
		printUsage(stdout)
		return 0
	default:
		printUsage(stdout)
		return 1
	}
}
