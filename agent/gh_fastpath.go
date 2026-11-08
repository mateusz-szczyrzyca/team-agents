package main

import (
	"io"
	"path/filepath"
)

// ghFastPath selects the wrapper by basename before configuration can execute.
func ghFastPath(argv0 string) bool {
	return filepath.Base(argv0) == "gh"
}

// runGH executes the native wrapper before configuration can run shell code.
func runGH(_ []string, _ []string, _ io.Writer, _ io.Writer) int {
	return 1
}
