package main

import (
	"context"
	"fmt"
	"io"
	"path/filepath"
)

// ghFastPath selects the wrapper by basename before configuration can execute.
func ghFastPath(argv0 string) bool {
	return filepath.Base(argv0) == "gh"
}

// runGH executes the native wrapper before configuration can run shell code.
func runGH(args []string, env []string, stdout io.Writer, stderr io.Writer) int {
	code, err := ghDispatch(context.Background(), GHDispatchParams{
		Args:   args,
		Env:    env,
		Stdout: stdout,
		Stderr: stderr,
		Exec:   OSExecutor{},
	})
	if err != nil {
		if err.Cause == GHReadDispatchInvalidJSON {
			if _, werr := fmt.Fprintf(stderr, "agent-gh: %s\n", err.Message); werr != nil {
				return 1
			}
		}
		if code == 0 {
			return 1
		}
		return code
	}
	return code
}
