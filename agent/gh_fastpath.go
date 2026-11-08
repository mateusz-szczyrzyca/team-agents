package main

import (
	"context"
	"fmt"
	"io"
	"os"
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
		Stdin:  os.Stdin,
		Stdout: stdout,
		Stderr: stderr,
		Exec:   OSExecutor{},
	})
	if err != nil {
		switch err.Cause {
		case GHReadDispatchInvalidJSON, GHReadDispatchStorage:
			if _, werr := fmt.Fprintf(stderr, "agent-gh: %s\n", err.Message); werr != nil {
				if code == 0 {
					return 1
				}
				return code
			}
		case GHReadDispatchExecFailed:
			// Process failure: subprocess already wrote its diagnostic to stderr; preserve its exit code.
		default:
			// Policy refusals (MissingRealGH, TemplateRefused, etc.) have already written their message via p.refuse.
		}
		if code == 0 {
			return 1
		}
		return code
	}
	return code
}
