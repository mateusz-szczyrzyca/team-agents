package main

import (
	"context"
	"io"
)

// dispatch routes a command to its handler, returning a typed CommandError on failure.
func dispatch(cmd string, args []string, stdout, stderr io.Writer) *CommandError {
	return newApp(nil, nil, nil).dispatch(context.Background(), cmd, args, stdout, stderr)
}

// dispatch routes a command within an App instance.
func (a *App) dispatch(ctx context.Context, cmd string, args []string, stdout, _ io.Writer) *CommandError {
	switch cmd {
	case "help", "-h", "--help":
		if err := printUsage(stdout); err != nil {
			return fromStorage(CommandStorage, storageErr(err, "print usage"))
		}
		return nil
	case "status", "ls":
		if err := cmdStatus(ctx, a, stdout); err != nil {
			return fromStorage(CommandStorage, storageErr(err.asError(), "status"))
		}
		return nil
	case "show":
		if len(args) == 0 {
			return opErr(CommandInvalidArgument, "usage: agent show <id>")
		}
		if err := cmdShow(ctx, a, args[0], stdout); err != nil {
			if err.Cause == ShowNotFound {
				return opErr(CommandInvalidArgument, "task %s not found", args[0])
			}
			return fromStorage(CommandStorage, storageErr(err.asError(), "show"))
		}
		return nil
	case "bus":
		var count string
		if len(args) > 0 {
			count = args[0]
		}
		return a.cmdBus(ctx, count, stdout)
	case "inv":
		return a.cmdInv(ctx, args, stdout)
	case "notes":
		return a.cmdNotes(ctx, args, stdout)
	case "helpers":
		return a.cmdHelpers(ctx, args, stdout)
	case "agents":
		return a.routeAgents(ctx, stdout)
	case "_agents-json":
		return a.routeAgentsJSON(ctx, stdout)
	case "ack":
		return a.routeAck(ctx, args, stdout)
	default:
		if err := printUsage(stdout); err != nil {
			return fromStorage(CommandStorage, storageErr(err, "print usage"))
		}
		return opErr(CommandUnknownCommand, "unknown command %q", cmd)
	}
}

// routeAgents delegates to cmdAgents or returns nil until merged.
func (a *App) routeAgents(_ context.Context, _ io.Writer) *CommandError {
	return nil
}

// routeAgentsJSON delegates to cmdAgentsJSON or returns nil until merged.
func (a *App) routeAgentsJSON(_ context.Context, _ io.Writer) *CommandError {
	return nil
}

// routeAck delegates to cmdAck or returns nil until merged.
func (a *App) routeAck(_ context.Context, _ []string, _ io.Writer) *CommandError {
	return nil
}

// run provides the CLI test entry point with external execution denied by default.
func run(args []string, stdout, stderr io.Writer) int {
	return newApp(nil, nil, nil).run(args, stdout, stderr)
}

// run routes the CLI within an owned application and preserves numeric exit codes.
func (a *App) run(args []string, stdout, stderr io.Writer) int {
	cmd := "help"
	var cmdArgs []string
	if len(args) > 0 {
		cmd = args[0]
		cmdArgs = args[1:]
	}

	if err := a.dispatch(context.Background(), cmd, cmdArgs, stdout, stderr); err != nil {
		return err.Class().ExitCode()
	}
	return 0
}
