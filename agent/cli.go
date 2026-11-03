package main

import (
	"io"
)

// dispatch routes a command to its handler, returning a typed CommandError on failure.
func dispatch(cmd string, _ []string, stdout, _ io.Writer) *CommandError {
	switch cmd {
	case "help", "-h", "--help":
		if err := printUsage(stdout); err != nil {
			return opErr(CommandInvalidArgument, "failed to print help: %v", err)
		}
		return nil
	default:
		_ = printUsage(stdout)
		return opErr(CommandUnknownCommand, "unknown command %q", cmd)
	}
}

// run executes the agent CLI command with the given arguments, writing output to stdout
// and diagnostics to stderr. It returns the process exit code.
func run(args []string, stdout, stderr io.Writer) int {
	cmd := "help"
	var cmdArgs []string
	if len(args) > 0 {
		cmd = args[0]
		cmdArgs = args[1:]
	}

	if err := dispatch(cmd, cmdArgs, stdout, stderr); err != nil {
		return err.Class().ExitCode()
	}
	return 0
}
