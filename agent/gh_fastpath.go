package main

import "path/filepath"

// ghFastPath reserves gh basename dispatch before configuration; native policies arrive in R11/R12.
func ghFastPath(argv0 string) *GHFastPathError {
	if filepath.Base(argv0) == "gh" {
		return opErr(GHFastPathUnavailable, "native GitHub policies are not implemented yet")
	}
	return nil
}
