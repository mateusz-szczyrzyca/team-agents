package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

// helperEnvKey is the environment variable that activates the test helper process.
const helperEnvKey = "GO_WANT_HELPER_PROCESS"

// TestHelperProcess serves as the target executable for subprocess tests.
func TestHelperProcess(_ *testing.T) {
	if os.Getenv(helperEnvKey) != "1" {
		return
	}
	args := os.Args
	for len(args) > 0 {
		if args[0] == "--" {
			args = args[1:]
			break
		}
		args = args[1:]
	}
	if len(args) == 0 {
		os.Exit(0)
	}

	cmd := args[0]
	switch cmd {
	case "echo-args":
		for i, arg := range args[1:] {
			if i > 0 {
				fmt.Print("\x00")
			}
			fmt.Print(arg)
		}
		os.Exit(0)
	case "print-env":
		key := args[1]
		fmt.Print(os.Getenv(key))
		os.Exit(0)
	case "count-env":
		fmt.Printf("%d", len(os.Environ()))
		os.Exit(0)
	case "print-cwd":
		cwd, err := os.Getwd()
		if err != nil {
			fmt.Fprintf(os.Stderr, "getwd: %v\n", err)
			os.Exit(1)
		}
		fmt.Print(cwd)
		os.Exit(0)
	case "cat-stdin":
		if _, err := io.Copy(os.Stdout, os.Stdin); err != nil {
			fmt.Fprintf(os.Stderr, "stdin copy: %v\n", err)
			os.Exit(1)
		}
		os.Exit(0)
	case "write-stderr":
		fmt.Fprint(os.Stderr, args[1])
		os.Exit(0)
	case "large-output":
		chunk := strings.Repeat("A", 64*1024)
		for i := 0; i < 16; i++ { // 1 MB total
			if _, err := os.Stdout.WriteString(chunk); err != nil {
				os.Exit(1)
			}
		}
		os.Exit(0)
	case "exit":
		var code int
		if _, err := fmt.Sscanf(args[1], "%d", &code); err != nil {
			fmt.Fprintf(os.Stderr, "sscanf exit code: %v\n", err)
			os.Exit(2)
		}
		os.Exit(code)
	case "signal-term":
		if err := syscall.Kill(syscall.Getpid(), syscall.SIGTERM); err != nil {
			fmt.Fprintf(os.Stderr, "kill: %v\n", err)
			os.Exit(1)
		}
		time.Sleep(1 * time.Second)
		os.Exit(0)
	case "sleep":
		time.Sleep(10 * time.Second)
		os.Exit(0)
	default:
		fmt.Fprintf(os.Stderr, "unknown helper command: %s\n", cmd)
		os.Exit(2)
	}
}

// helperCmd constructs a Command that invokes TestHelperProcess with the given arguments.
func helperCmd(args ...string) Command {
	cmdArgs := append([]string{"-test.run=^TestHelperProcess$", "--"}, args...)
	return Command{
		Name: os.Args[0],
		Args: cmdArgs,
		Env:  append(os.Environ(), helperEnvKey+"=1"),
	}
}

// failWriter is an io.Writer that always returns an error on Write.
type failWriter struct {
	err error // error returned on write
}

// Write returns the configured failure error.
func (f failWriter) Write(_ []byte) (int, error) {
	return 0, f.err
}

// TestDeniedExecutorFailsClosed tests that deniedExecutor always returns ExecuteDenied.
func TestDeniedExecutorFailsClosed(t *testing.T) {
	exec := newDeniedExecutor()
	cmd := Command{Name: "echo", Args: []string{"hello"}}
	_, err := exec.Execute(context.Background(), cmd)
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if err.Cause != ExecuteDenied {
		t.Fatalf("expected cause ExecuteDenied, got %v", err.Cause)
	}
	if !errors.Is(err, ErrPermission) {
		t.Fatalf("expected ErrPermission, got %v", err)
	}
}

// TestExecuteInvalidCommand tests that an empty command name produces ExecuteInvalidCommand.
func TestExecuteInvalidCommand(t *testing.T) {
	exec := OSExecutor{}
	cmd := Command{Name: ""}
	_, err := exec.Execute(context.Background(), cmd)
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if err.Cause != ExecuteInvalidCommand {
		t.Fatalf("expected cause ExecuteInvalidCommand, got %v", err.Cause)
	}
	if !errors.Is(err, ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument, got %v", err)
	}
}

// TestExecuteFailedStart tests that a nonexistent binary produces ExecuteStart.
func TestExecuteFailedStart(t *testing.T) {
	exec := OSExecutor{}
	cmd := Command{Name: "/nonexistent/binary/path/which/does/not/exist"}
	_, err := exec.Execute(context.Background(), cmd)
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if err.Cause != ExecuteStart {
		t.Fatalf("expected cause ExecuteStart, got %v", err.Cause)
	}
	if !errors.Is(err, ErrProcess) {
		t.Fatalf("expected ErrProcess, got %v", err)
	}
}

// TestExecuteArgvPreservation tests argument preservation with spaces, newlines, and metacharacters.
func TestExecuteArgvPreservation(t *testing.T) {
	exec := OSExecutor{}
	rawArgs := []string{
		"arg with spaces",
		"line1\nline2",
		"$VAR && | ; < > ' \" * ? ~ `",
		"trailing space ",
		"",
	}
	var stdout bytes.Buffer
	cmd := helperCmd(append([]string{"echo-args"}, rawArgs...)...)
	cmd.Stdout = &stdout

	res, err := exec.Execute(context.Background(), cmd)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res.ExitCode != 0 {
		t.Fatalf("expected exit code 0, got %d", res.ExitCode)
	}

	parts := strings.Split(stdout.String(), "\x00")
	if len(parts) != len(rawArgs) {
		t.Fatalf("expected %d args, got %d: %q", len(rawArgs), len(parts), stdout.String())
	}
	for i, expected := range rawArgs {
		if parts[i] != expected {
			t.Errorf("arg %d: expected %q, got %q", i, expected, parts[i])
		}
	}
}

// TestExecuteEnvironment tests environment inheritance, replacement, and clearing.
func TestExecuteEnvironment(t *testing.T) {
	exec := OSExecutor{}

	// Case 1: nil inherits current environment
	t.Run("nil inherits", func(t *testing.T) {
		t.Setenv("TEST_INHERITED_VAR", "inherited_val_123")
		var stdout bytes.Buffer
		cmd := helperCmd("print-env", "TEST_INHERITED_VAR")
		cmd.Stdout = &stdout
		cmd.Env = nil

		res, err := exec.Execute(context.Background(), cmd)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if res.ExitCode != 0 {
			t.Fatalf("expected exit code 0, got %d", res.ExitCode)
		}
		if stdout.String() != "inherited_val_123" {
			t.Fatalf("expected inherited_val_123, got %q", stdout.String())
		}
	})

	// Case 2: non-nil replaces environment
	t.Run("non-nil replaces", func(t *testing.T) {
		t.Setenv("TEST_UNSEEN_VAR", "unseen")
		var stdout bytes.Buffer
		cmd := helperCmd("print-env", "TEST_EXPLICIT_VAR")
		cmd.Stdout = &stdout
		cmd.Env = []string{helperEnvKey + "=1", "TEST_EXPLICIT_VAR=explicit_val_456"}

		res, err := exec.Execute(context.Background(), cmd)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if res.ExitCode != 0 {
			t.Fatalf("expected exit code 0, got %d", res.ExitCode)
		}
		if stdout.String() != "explicit_val_456" {
			t.Fatalf("expected explicit_val_456, got %q", stdout.String())
		}
	})

	// Case 3: empty non-nil slice clears environment
	t.Run("empty slice clears", func(t *testing.T) {
		var stdout bytes.Buffer
		cmd := helperCmd("count-env")
		cmd.Stdout = &stdout
		// Provide only the helper activation key so the helper can run
		cmd.Env = []string{helperEnvKey + "=1"}

		res, err := exec.Execute(context.Background(), cmd)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if res.ExitCode != 0 {
			t.Fatalf("expected exit code 0, got %d", res.ExitCode)
		}
		if stdout.String() != "1" {
			t.Fatalf("expected exactly 1 env var, got count %q", stdout.String())
		}
	})
}

// TestExecuteWorkingDirectory tests running with a specific working directory.
func TestExecuteWorkingDirectory(t *testing.T) {
	exec := OSExecutor{}
	tempDir := t.TempDir()
	realTemp, err := filepath.EvalSymlinks(tempDir)
	if err != nil {
		t.Fatalf("eval symlinks: %v", err)
	}

	var stdout bytes.Buffer
	cmd := helperCmd("print-cwd")
	cmd.Dir = tempDir
	cmd.Stdout = &stdout

	res, execErr := exec.Execute(context.Background(), cmd)
	if execErr != nil {
		t.Fatalf("unexpected error: %v", execErr)
	}
	if res.ExitCode != 0 {
		t.Fatalf("expected exit code 0, got %d", res.ExitCode)
	}
	gotDir, err := filepath.EvalSymlinks(stdout.String())
	if err != nil {
		t.Fatalf("eval symlinks got: %v", err)
	}
	if gotDir != realTemp {
		t.Fatalf("expected cwd %q, got %q", realTemp, gotDir)
	}
}

// TestExecuteIOStreams tests stdin streaming and stderr capturing.
func TestExecuteIOStreams(t *testing.T) {
	exec := OSExecutor{}
	stdinData := "piped input payload with special chars: \n\t$#@"
	stderrData := "diagnostic log line to stderr"

	var stdout bytes.Buffer
	var stderr bytes.Buffer

	// Stdin -> Stdout
	cmdIn := helperCmd("cat-stdin")
	cmdIn.Stdin = strings.NewReader(stdinData)
	cmdIn.Stdout = &stdout

	res, err := exec.Execute(context.Background(), cmdIn)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res.ExitCode != 0 {
		t.Fatalf("expected exit code 0, got %d", res.ExitCode)
	}
	if stdout.String() != stdinData {
		t.Fatalf("expected stdin payload, got %q", stdout.String())
	}

	// Stderr capture
	cmdErr := helperCmd("write-stderr", stderrData)
	cmdErr.Stderr = &stderr

	res, err = exec.Execute(context.Background(), cmdErr)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res.ExitCode != 0 {
		t.Fatalf("expected exit code 0, got %d", res.ExitCode)
	}
	if stderr.String() != stderrData {
		t.Fatalf("expected stderr payload, got %q", stderr.String())
	}
}

// TestExecuteLargeOutput tests streaming large outputs without hanging or truncation.
func TestExecuteLargeOutput(t *testing.T) {
	exec := OSExecutor{}
	var stdout bytes.Buffer

	cmd := helperCmd("large-output")
	cmd.Stdout = &stdout

	res, err := exec.Execute(context.Background(), cmd)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res.ExitCode != 0 {
		t.Fatalf("expected exit code 0, got %d", res.ExitCode)
	}
	if stdout.Len() != 1024*1024 {
		t.Fatalf("expected 1MB output, got %d bytes", stdout.Len())
	}
}

// TestExecuteExitCode tests capturing non-zero exit codes with nil error.
func TestExecuteExitCode(t *testing.T) {
	exec := OSExecutor{}

	cmd := helperCmd("exit", "42")
	res, err := exec.Execute(context.Background(), cmd)
	if err != nil {
		t.Fatalf("expected nil error on nonzero exit, got %v", err)
	}
	if res.ExitCode != 42 {
		t.Fatalf("expected exit code 42, got %d", res.ExitCode)
	}
}

// TestExecuteSignaled tests that signal termination maps to 128+signal with nil error.
func TestExecuteSignaled(t *testing.T) {
	exec := OSExecutor{}

	cmd := helperCmd("signal-term")
	res, err := exec.Execute(context.Background(), cmd)
	if err != nil {
		t.Fatalf("expected nil error on signaled exit, got %v", err)
	}
	expectedCode := 128 + int(syscall.SIGTERM)
	if res.ExitCode != expectedCode {
		t.Fatalf("expected exit code %d (128+SIGTERM), got %d", expectedCode, res.ExitCode)
	}
}

// TestExecuteContextCancellation tests that cancelling the context returns ExecuteWait.
func TestExecuteContextCancellation(t *testing.T) {
	exec := OSExecutor{}

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()

	cmd := helperCmd("sleep")
	_, err := exec.Execute(ctx, cmd)
	if err == nil {
		t.Fatal("expected error on cancelled context, got nil")
	}
	if err.Cause != ExecuteWait {
		t.Fatalf("expected cause ExecuteWait, got %v", err.Cause)
	}
	if !errors.Is(err, ErrProcess) {
		t.Fatalf("expected ErrProcess, got %v", err)
	}
}

// TestExecuteWriterFailure tests that a failing writer during execution returns ExecuteWait.
func TestExecuteWriterFailure(t *testing.T) {
	exec := OSExecutor{}

	writerErr := errors.New("broken output pipe")
	cmd := helperCmd("large-output")
	cmd.Stdout = failWriter{err: writerErr}

	_, err := exec.Execute(context.Background(), cmd)
	if err == nil {
		t.Fatal("expected error on failing writer, got nil")
	}
	if err.Cause != ExecuteWait {
		t.Fatalf("expected cause ExecuteWait, got %v", err.Cause)
	}
	if !errors.Is(err, ErrProcess) {
		t.Fatalf("expected ErrProcess, got %v", err)
	}
}

// TestFromProcessConversion verifies fromProcess converts *ProcessError into the operation cause.
func TestFromProcessConversion(t *testing.T) {
	underlying := errors.New("underlying failure")
	perr := processErr("cmd", underlying, "exec failed")

	opErr := fromProcess(ExecuteStart, perr)
	if opErr == nil {
		t.Fatal("expected non-nil OpError, got nil")
	}
	if opErr.Cause != ExecuteStart {
		t.Fatalf("expected cause ExecuteStart, got %v", opErr.Cause)
	}
	if !errors.Is(opErr, ErrProcess) {
		t.Fatalf("expected ErrProcess, got %v", opErr)
	}
	if !errors.Is(opErr, underlying) {
		t.Fatalf("expected underlying error, got %v", opErr)
	}
	var extracted *ProcessError
	if !errors.As(opErr, &extracted) || extracted != perr {
		t.Fatalf("expected extracted ProcessError, got %v", extracted)
	}
	if fromProcess(ExecuteStart, nil) != nil {
		t.Fatal("nil ProcessError should convert to nil OpError")
	}
}
