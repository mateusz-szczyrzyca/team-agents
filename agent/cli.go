package main

import (
	"context"
	"errors"
	"fmt"
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
			if err.Cause == StatusProcess {
				return opErr(CommandProcess, "%s", err.Message)
			}
			return fromStorage(CommandStorage, storageErr(err.asError(), "status"))
		}
		return nil
	case "show":
		target := firstArg(args)
		if target == "" {
			return opErr(CommandInvalidArgument, "usage: agent show <id>")
		}
		if err := cmdShow(ctx, a, target, stdout); err != nil {
			switch err.Cause {
			case ShowNotFound:
				return opErr(CommandNotFound, "%s", err.Message)
			case ShowAmbiguous:
				return opErr(CommandConflict, "%s", err.Message)
			default:
				return &CommandError{Cause: CommandStorage, Message: err.Message, Err: errors.Join(CommandStorage.info().class.Sentinel(), err)}
			}
		}
		return nil
	case "bus":
		return a.cmdBus(ctx, firstArg(args), stdout)
	case "inv":
		return a.cmdInv(ctx, args, stdout)
	case "notes":
		return a.cmdNotes(ctx, args, stdout)
	case "helpers":
		return a.cmdHelpers(ctx, args, stdout)
	case "agents":
		return a.routeAgents(ctx, stdout)
	case "_agents-json":
		if err := a.cmdAgentsJSON(stdout, a.listingConfig()); err != nil {
			return fromStorage(CommandStorage, storageErr(err.asError(), "agents json"))
		}
		return nil
	case "ack":
		target := firstArg(args)
		if target == "" {
			return opErr(CommandInvalidArgument, "usage: agent ack <id|all>")
		}
		if err := a.cmdAck(target, stdout, a.tasksDirectory()); err != nil {
			switch err.Cause {
			case AckNotFound:
				return opErr(CommandNotFound, "%s", err.Message)
			case AckAmbiguous, AckStillRunning:
				return opErr(CommandConflict, "%s", err.Message)
			default:
				return fromStorage(CommandStorage, storageErr(err.asError(), "ack"))
			}
		}
		return nil
	default:
		if err := printUsage(stdout); err != nil {
			return fromStorage(CommandStorage, storageErr(err, "print usage"))
		}
		return opErr(CommandUnknownCommand, "unknown command %q", cmd)
	}
}

// routeAgents delegates to cmdAgents.
func (a *App) routeAgents(_ context.Context, stdout io.Writer) *CommandError {
	if err := a.cmdAgents(stdout, a.listingConfig(), a.tasksDirectory(), a.busDirectory()); err != nil {
		if err.Cause == AgentsProcess {
			return &CommandError{Cause: CommandProcess, Message: err.Message, Err: errors.Join(CommandProcess.info().class.Sentinel(), err)}
		}
		return fromStorage(CommandStorage, storageErr(err.asError(), "agents"))
	}
	return nil
}

// firstArg returns the first element of args or empty string.
func firstArg(args []string) string {
	if len(args) > 0 {
		return args[0]
	}
	return ""
}

// run provides the CLI test entry point with external execution denied by default.
func run(args []string, stdout, stderr io.Writer) int {
	return newApp(nil, nil, nil).run(args, stdout, stderr)
}

// run routes the CLI within an owned application and preserves numeric exit codes.
func (a *App) run(args []string, stdout, stderr io.Writer) int {
	a.Stderr = stderr
	cmd := "help"
	var cmdArgs []string
	if len(args) > 0 {
		cmd = args[0]
		cmdArgs = args[1:]
	}

	if err := a.dispatch(context.Background(), cmd, cmdArgs, stdout, stderr); err != nil {
		var berr *BusError
		if errors.As(err, &berr) {
			if isLaunchError(berr) {
				if _, werr := fmt.Fprintf(stderr, "agent: %s\n", err.Message); werr != nil {
					return 1
				}
			}
			return busExitCode(berr)
		}
		if err.Cause != CommandUnknownCommand {
			if _, werr := fmt.Fprintf(stderr, "agent: %s\n", err.Message); werr != nil {
				return 1
			}
		}
		return err.Class().ExitCode()
	}
	return 0
}
