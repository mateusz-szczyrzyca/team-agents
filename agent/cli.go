package main

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
)

// dispatch routes a command to its handler, returning a typed CommandError on failure.
func dispatch(cmd string, args []string, stdout, stderr io.Writer) *CommandError {
	return newApp(nil, nil, nil).dispatch(context.Background(), cmd, args, stdout, stderr)
}

// dispatch routes a command within an App instance.
func (a *App) dispatch(ctx context.Context, cmd string, args []string, stdout, stderr io.Writer) *CommandError {
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
		if err := cmdShow(ctx, a, args[0], stdout, stderr); err != nil {
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

// routeAgents delegates to cmdAgents.
func (a *App) routeAgents(_ context.Context, stdout io.Writer) *CommandError {
	tasksDir := a.tasksDir
	if tasksDir == "" {
		home := a.agentHome
		if home == "" {
			home = os.Getenv("HOME")
		}
		tasksDir = filepath.Join(home, "agents", "tasks")
	}
	busDir := a.busDirectory()
	cfg := &Config{
		Scalars: map[string]string{
			"AGENTBUS_BIN": a.busBinary(),
		},
	}
	if err := a.cmdAgents(stdout, cfg, tasksDir, busDir); err != nil {
		return fromStorage(CommandStorage, storageErr(err.asError(), "agents"))
	}
	return nil
}

// routeAgentsJSON delegates to cmdAgentsJSON.
func (a *App) routeAgentsJSON(_ context.Context, stdout io.Writer) *CommandError {
	cfg := &Config{}
	if err := a.cmdAgentsJSON(stdout, cfg); err != nil {
		return fromStorage(CommandStorage, storageErr(err.asError(), "agents json"))
	}
	return nil
}

// routeAck delegates to cmdAck.
func (a *App) routeAck(_ context.Context, args []string, stdout io.Writer) *CommandError {
	if len(args) == 0 {
		return opErr(CommandInvalidArgument, "usage: agent ack <id|all>")
	}
	tasksDir := a.tasksDir
	if tasksDir == "" {
		home := a.agentHome
		if home == "" {
			home = os.Getenv("HOME")
		}
		tasksDir = filepath.Join(home, "agents", "tasks")
	}
	if err := a.cmdAck(args[0], stdout, tasksDir); err != nil {
		if err.Cause == AckNotFound || err.Cause == AckAmbiguous || err.Cause == AckStillRunning {
			return opErr(CommandInvalidArgument, "%s", err.Message)
		}
		return fromStorage(CommandStorage, storageErr(err.asError(), "ack"))
	}
	return nil
}

// run provides the CLI test entry point with external execution denied by default.
func run(args []string, stdout, stderr io.Writer) int {
	return newApp(nil, nil, nil).run(args, stdout, stderr)
}

// run routes the CLI within an owned application and preserves numeric exit codes.
func (a *App) run(args []string, stdout, stderr io.Writer) int {
	a.Stdout = stdout
	a.Stderr = stderr

	cmd := "help"
	var cmdArgs []string
	if len(args) > 0 {
		cmd = args[0]
		cmdArgs = args[1:]
	}

	if err := a.dispatch(context.Background(), cmd, cmdArgs, stdout, stderr); err != nil {
		// Check for OS exit codes 126 (permission denied) and 127 (not found) in process errors.
		if strings.Contains(err.Message, "exit code 126") {
			return 126
		} else if strings.Contains(err.Message, "exit code 127") {
			return 127
		}
		return err.Class().ExitCode()
	}
	return 0
}
