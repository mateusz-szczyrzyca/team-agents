package main

import (
	"context"
	"errors"
	"io"
	"os/exec"
	"syscall"
)

// Command specifies a synchronous external command to execute.
type Command struct {
	Name   string    // path or executable name to run
	Args   []string  // command arguments (excluding argv[0] / Name)
	Env    []string  // environment variables (nil inherits current process environment, non-nil replaces)
	Dir    string    // working directory for the command (empty string inherits current directory)
	Stdin  io.Reader // standard input stream for the command (nil connects to no input)
	Stdout io.Writer // destination for standard output (nil discards)
	Stderr io.Writer // destination for standard error (nil discards)
}

// CommandResult holds the result of a completed command execution.
type CommandResult struct {
	ExitCode int // process exit code (0 for success, 1-255 for non-zero exit, 128+signal for signaled termination)
}

// Executor runs commands synchronously within a context.
type Executor interface {
	Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError)
}

// OSExecutor executes commands via the operating system using os/exec.
type OSExecutor struct{}

// Execute executes the command synchronously using os/exec.
func (OSExecutor) Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	if cmd.Name == "" {
		return CommandResult{}, opErr(ExecuteInvalidCommand, "command name cannot be empty")
	}

	c := exec.CommandContext(ctx, cmd.Name, cmd.Args...) // #nosec G204 -- synchronous execution of caller-provided command and argv without shell interpolation
	if cmd.Env != nil {
		c.Env = cmd.Env
	}
	if cmd.Dir != "" {
		c.Dir = cmd.Dir
	}
	c.Stdin = cmd.Stdin
	c.Stdout = cmd.Stdout
	c.Stderr = cmd.Stderr

	if err := c.Start(); err != nil {
		return CommandResult{}, fromProcess(ExecuteStart, processErr(cmd.Name, err, "failed to start %s", cmd.Name))
	}

	err := c.Wait()
	if ctx.Err() != nil {
		return CommandResult{}, fromProcess(ExecuteWait, processErr(cmd.Name, ctx.Err(), "command canceled: %s", cmd.Name))
	}
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			if ws, ok := exitErr.Sys().(syscall.WaitStatus); ok && ws.Signaled() {
				return CommandResult{ExitCode: 128 + int(ws.Signal())}, nil
			}
			return CommandResult{ExitCode: exitErr.ExitCode()}, nil
		}
		return CommandResult{}, fromProcess(ExecuteWait, processErr(cmd.Name, err, "wait failed for %s", cmd.Name))
	}

	return CommandResult{ExitCode: 0}, nil
}

// deniedExecutor is a fail-closed executor that rejects all executions.
type deniedExecutor struct{}

// newDeniedExecutor returns a fail-closed executor that denies all commands.
func newDeniedExecutor() Executor {
	return deniedExecutor{}
}

// Execute rejects execution with an ExecuteDenied error.
func (deniedExecutor) Execute(_ context.Context, cmd Command) (CommandResult, *ExecuteError) {
	return CommandResult{}, opErr(ExecuteDenied, "command execution is denied by policy: %s", cmd.Name)
}
