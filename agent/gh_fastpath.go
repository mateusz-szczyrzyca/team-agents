package main

import (
	"context"
	"io"
	"path/filepath"
)

// ghFastPath selects the wrapper by basename before configuration can execute.
func ghFastPath(argv0 string) bool {
	return filepath.Base(argv0) == "gh"
}

// runGH executes the native wrapper before configuration can run shell code.
func runGH(args []string, env []string, stdout io.Writer, stderr io.Writer) int {
	code, _ := ghDispatch(context.Background(), GHDispatchParams{
		Args:   args,
		Env:    env,
		Stdout: stdout,
		Stderr: stderr,
		Exec:   OSExecutor{},
	})
	return code
}
