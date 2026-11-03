package main

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"sync"
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

// streamErrorWriter tracks errors occurring during writes to an underlying io.Writer.
type streamErrorWriter struct {
	w   io.Writer  // underlying writer destination
	mu  sync.Mutex // guards access to err and writes
	err error      // first write error encountered
}

// Write writes to the underlying writer while recording any error.
func (s *streamErrorWriter) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	n, err := s.w.Write(p)
	if err != nil && s.err == nil {
		s.err = err
	}
	return n, err
}

// error returns the recorded write error, if any.
func (s *streamErrorWriter) error() error {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.err
}

// streamErrorReader tracks errors occurring during reads from an underlying io.Reader.
type streamErrorReader struct {
	r   io.Reader  // underlying reader source
	mu  sync.Mutex // guards access to err
	err error      // first read error encountered (excluding EOF)
}

// Read reads from the underlying reader while recording any non-EOF error.
func (s *streamErrorReader) Read(p []byte) (int, error) {
	n, err := s.r.Read(p)
	if err != nil && !errors.Is(err, io.EOF) {
		s.mu.Lock()
		if s.err == nil {
			s.err = err
		}
		s.mu.Unlock()
	}
	return n, err
}

// error returns the recorded read error, if any.
func (s *streamErrorReader) error() error {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.err
}

// streamError collects any stream errors captured during process execution.
func streamError(stdin *streamErrorReader, stdout, stderr *streamErrorWriter) error {
	var errs []error
	if stdin != nil {
		if err := stdin.error(); err != nil {
			errs = append(errs, err)
		}
	}
	if stdout != nil {
		if err := stdout.error(); err != nil {
			errs = append(errs, err)
		}
	}
	if stderr != nil && stderr != stdout {
		if err := stderr.error(); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

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

	var stdinTracker *streamErrorReader
	if cmd.Stdin != nil {
		if _, ok := cmd.Stdin.(*os.File); ok {
			c.Stdin = cmd.Stdin
		} else {
			stdinTracker = &streamErrorReader{r: cmd.Stdin}
			c.Stdin = stdinTracker
		}
	}

	var stdoutTracker *streamErrorWriter
	var stderrTracker *streamErrorWriter
	if cmd.Stdout != nil && cmd.Stdout == cmd.Stderr {
		if _, ok := cmd.Stdout.(*os.File); ok {
			c.Stdout = cmd.Stdout
			c.Stderr = cmd.Stderr
		} else {
			tracker := &streamErrorWriter{w: cmd.Stdout}
			stdoutTracker = tracker
			stderrTracker = tracker
			c.Stdout = tracker
			c.Stderr = tracker
		}
	} else {
		if cmd.Stdout != nil {
			if _, ok := cmd.Stdout.(*os.File); ok {
				c.Stdout = cmd.Stdout
			} else {
				stdoutTracker = &streamErrorWriter{w: cmd.Stdout}
				c.Stdout = stdoutTracker
			}
		}
		if cmd.Stderr != nil {
			if _, ok := cmd.Stderr.(*os.File); ok {
				c.Stderr = cmd.Stderr
			} else {
				stderrTracker = &streamErrorWriter{w: cmd.Stderr}
				c.Stderr = stderrTracker
			}
		}
	}

	if err := c.Start(); err != nil {
		return CommandResult{}, fromProcess(ExecuteStart, processErr(cmd.Name, err, "failed to start %s", cmd.Name))
	}

	err := c.Wait()
	if ctx.Err() != nil {
		return CommandResult{}, fromProcess(ExecuteWait, processErr(cmd.Name, ctx.Err(), "command canceled: %s", cmd.Name))
	}
	if streamErr := streamError(stdinTracker, stdoutTracker, stderrTracker); streamErr != nil {
		return CommandResult{}, fromProcess(ExecuteWait, processErr(cmd.Name, streamErr, "stream I/O failed for %s", cmd.Name))
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
