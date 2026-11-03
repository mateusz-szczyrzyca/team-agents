package main

import (
	"io"
)

// dispatch routes a command to its handler, returning a typed CommandError on failure.
func dispatch(cmd string, _ []string, stdout, _ io.Writer) *CommandError {
	switch cmd {
	case "help", "-h", "--help":
		if err := printUsage(stdout); err != nil {
			return fromStorage(CommandStorage, err)
		}
		return nil
	default:
		if err := printUsage(stdout); err != nil {
			return fromStorage(CommandStorage, err)
		}
		return opErr(CommandUnknownCommand, "unknown command %q", cmd)
	}
}

// run provides the CLI test entry point with external execution denied by default.
func run(args []string, stdout, stderr io.Writer) int {
	return newApp(nil, nil, nil).run(args, stdout, stderr)
}

// run routes the CLI within an owned application and preserves numeric exit codes.
func (*App) run(args []string, stdout, stderr io.Writer) int {
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
