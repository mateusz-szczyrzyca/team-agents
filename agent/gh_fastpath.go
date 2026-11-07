package main

// ghFastPath reserves gh basename dispatch before configuration; native policies arrive in R11/R12.
func ghFastPath(_ string) *GHFastPathError {
	return nil
}
