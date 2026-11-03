package main

import (
	"io"
)

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
