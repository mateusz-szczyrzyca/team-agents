package main

import (
	"os"
)

// main is the entry point for the Go implementation of the agent binary.
func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}
