package main

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
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
	w       io.Writer   // underlying writer destination
	writeMu *sync.Mutex // serializes concurrent writes across output streams
	errMu   sync.Mutex  // guards access to err
	err     error       // first write error encountered
}

// Write writes to the underlying writer while recording any error.
func (s *streamErrorWriter) Write(p []byte) (int, error) {
	s.writeMu.Lock()
	n, err := s.w.Write(p)
	s.writeMu.Unlock()
	if err != nil || n != len(p) {
		s.errMu.Lock()
		if s.err == nil {
			if err != nil {
				s.err = err
			} else {
				s.err = io.ErrShortWrite
			}
		}
		s.errMu.Unlock()
	}
	return n, err
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
	if err != nil && err != io.EOF {
		s.mu.Lock()
		if s.err == nil {
			s.err = err
		}
		s.mu.Unlock()
	}
	return n, err
}

// collectStreamError collects any stream errors captured during process execution as a *ProcessError.
func collectStreamError(program string, stdin *streamErrorReader, stdout, stderr *streamErrorWriter) *ProcessError {
	var errs []error
	if stdin != nil {
		stdin.mu.Lock()
		if stdin.err != nil {
			errs = append(errs, stdin.err)
		}
		stdin.mu.Unlock()
	}
	if stdout != nil {
		stdout.errMu.Lock()
		if stdout.err != nil {
			errs = append(errs, stdout.err)
		}
		stdout.errMu.Unlock()
	}
	if stderr != nil && stderr != stdout {
		stderr.errMu.Lock()
		if stderr.err != nil {
			errs = append(errs, stderr.err)
		}
		stderr.errMu.Unlock()
	}
	if len(errs) == 0 {
		return nil
	}
	return processErr(program, errors.Join(errs...), "stream I/O failed for %s", program)
}

// sameWriter reports whether two writers are equal, avoiding panics if dynamic types are non-comparable.
func sameWriter(a, b io.Writer) bool {
	if a == nil || b == nil {
		return a == b
	}
	tA := reflect.TypeOf(a)
	tB := reflect.TypeOf(b)
	if tA != tB || !reflect.ValueOf(a).Comparable() {
		return false
	}
	return a == b
}

// lookPathEnv resolves a bare executable name in pathVal, returning the path or an error.
func lookPathEnv(file, pathVal string) (string, error) {
	var permErr error
	for _, dir := range filepath.SplitList(pathVal) {
		if dir == "" {
			dir = "."
		}
		target := filepath.Join(dir, file)
		if fi, err := os.Stat(target); err == nil && !fi.IsDir() {
			if fi.Mode()&0111 != 0 {
				return target, nil
			}
			if permErr == nil {
				permErr = &os.PathError{Op: "stat", Path: target, Err: os.ErrPermission}
			}
		}
	}
	if permErr != nil {
		return "", permErr
	}
	return "", &exec.Error{Name: file, Err: exec.ErrNotFound}
}

// Execute executes the command synchronously using os/exec.
func (OSExecutor) Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	if cmd.Name == "" {
		return CommandResult{}, opErr(ExecuteInvalidCommand, "command name cannot be empty")
	}

	target := cmd.Name
	if !strings.Contains(cmd.Name, "/") && cmd.Env != nil {
		for i := len(cmd.Env) - 1; i >= 0; i-- {
			if strings.HasPrefix(cmd.Env[i], "PATH=") {
				resolved, err := lookPathEnv(cmd.Name, cmd.Env[i][5:])
				if err != nil {
					return CommandResult{}, fromProcess(ExecuteStart, processErr(cmd.Name, err, "failed to start %s", cmd.Name))
				}
				target = resolved
				break
			}
		}
	}

	c := exec.CommandContext(ctx, target, cmd.Args...) // #nosec G204 -- synchronous execution of caller-provided command and argv without shell interpolation
	c.Args[0] = cmd.Name
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

	var writeMu sync.Mutex
	var stdoutTracker *streamErrorWriter
	var stderrTracker *streamErrorWriter
	wrapOutput := func(w io.Writer) (*streamErrorWriter, io.Writer) {
		if w == nil {
			return nil, nil
		}
		if _, ok := w.(*os.File); ok {
			return nil, w
		}
		tracker := &streamErrorWriter{w: w, writeMu: &writeMu}
		return tracker, tracker
	}
	if cmd.Stdout != nil && sameWriter(cmd.Stdout, cmd.Stderr) {
		stdoutTracker, c.Stdout = wrapOutput(cmd.Stdout)
		stderrTracker, c.Stderr = stdoutTracker, c.Stdout
	} else {
		stdoutTracker, c.Stdout = wrapOutput(cmd.Stdout)
		stderrTracker, c.Stderr = wrapOutput(cmd.Stderr)
	}

	if err := c.Start(); err != nil {
		return CommandResult{}, fromProcess(ExecuteStart, processErr(cmd.Name, err, "failed to start %s", cmd.Name))
	}

	err := c.Wait()
	if ctx.Err() != nil {
		return CommandResult{}, fromProcess(ExecuteWait, processErr(cmd.Name, ctx.Err(), "command canceled: %s", cmd.Name))
	}
	if streamErr := collectStreamError(cmd.Name, stdinTracker, stdoutTracker, stderrTracker); streamErr != nil {
		return CommandResult{}, fromProcess(ExecuteWait, streamErr)
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
