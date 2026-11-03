package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strconv"
	"syscall"
	"testing"
	"time"
)

// reapingHelperEnv activates only the dedicated child in this test binary.
const reapingHelperEnv = "AGENT_TEST_REAPING_CHILD"

// TestReapingChild announces readiness before blocking on its parent's pipe.
func TestReapingChild(_ *testing.T) {
	if os.Getenv(reapingHelperEnv) != "1" {
		return
	}
	if _, err := os.Stdout.WriteString(strconv.Itoa(os.Getpid()) + "\n"); err != nil {
		os.Exit(2)
	}
	if _, err := io.Copy(io.Discard, os.Stdin); err != nil {
		os.Exit(2)
	}
	os.Exit(0)
}

// TestExecuteCancellationReapsChild verifies cleanup after a readiness handshake.
func TestExecuteCancellationReapsChild(t *testing.T) {
	self, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	input, inputWriter, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		for _, file := range []*os.File{input, inputWriter} {
			if closeErr := file.Close(); closeErr != nil {
				t.Error(closeErr)
			}
		}
	})
	output, outputWriter, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		for _, file := range []*os.File{output, outputWriter} {
			if closeErr := file.Close(); closeErr != nil && !errors.Is(closeErr, os.ErrClosed) {
				t.Error(closeErr)
			}
		}
	})
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	done := make(chan *ExecuteError, 1)
	go func() {
		_, execErr := (OSExecutor{}).Execute(ctx, Command{Name: self,
			Args: []string{"-test.run=^TestReapingChild$"},
			Env:  []string{reapingHelperEnv + "=1"}, Stdin: input, Stdout: outputWriter})
		// Closing our write end also unblocks the handshake if execution fails.
		if closeErr := outputWriter.Close(); closeErr != nil {
			t.Error(closeErr)
		}
		done <- execErr
	}()
	var pid int
	if _, scanErr := fmt.Fscanln(output, &pid); scanErr != nil {
		t.Fatalf("child readiness handshake failed: %v", scanErr)
	}
	t.Cleanup(func() {
		if signalErr := syscall.Kill(pid, 0); errors.Is(signalErr, syscall.ESRCH) {
			return
		}
		// Reap an unexpectedly retained child even after a failed assertion.
		if killErr := syscall.Kill(pid, syscall.SIGKILL); killErr != nil && !errors.Is(killErr, syscall.ESRCH) {
			t.Error(killErr)
		}
		if _, waitErr := syscall.Wait4(pid, nil, 0, nil); waitErr != nil && !errors.Is(waitErr, syscall.ECHILD) {
			t.Error(waitErr)
		}
	})
	cancel()
	execErr := <-done
	if execErr == nil || execErr.Cause != ExecuteWait || !errors.Is(execErr, context.Canceled) {
		t.Fatalf("cancellation result: %v", execErr)
	}
	if signalErr := syscall.Kill(pid, 0); !errors.Is(signalErr, syscall.ESRCH) {
		t.Errorf("child %d was not reaped: %v", pid, signalErr)
	}
}
