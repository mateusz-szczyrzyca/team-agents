// app.go contains the application structure and owned dependencies.
package main

import (
	"io"
)

// App owns the dependencies of the agent CLI: paths, configuration, output streams,
// process execution, clock, random source, and runner log sink.
type App struct {
	// stdout is the standard output stream for the application.
	stdout io.Writer
	// stderr is the standard error stream for the application.
	stderr io.Writer
}

// NewApp creates a new App instance with the given output streams.
func NewApp(stdout, stderr io.Writer) *App {
	return &App{
		stdout: stdout,
		stderr: stderr,
	}
}
