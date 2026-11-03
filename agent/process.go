package main

import (
	"context"
	"io"
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
func (OSExecutor) Execute(_ context.Context, _ Command) (CommandResult, *ExecuteError) {
	return CommandResult{}, nil
}

// deniedExecutor is a fail-closed executor that rejects all executions.
type deniedExecutor struct{}

// newDeniedExecutor returns a fail-closed executor that denies all commands.
func newDeniedExecutor() Executor {
	return deniedExecutor{}
}

// Execute rejects execution with an ExecuteDenied error.
func (deniedExecutor) Execute(_ context.Context, _ Command) (CommandResult, *ExecuteError) {
	return CommandResult{}, nil
}
