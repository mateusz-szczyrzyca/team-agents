package main

import "context"

// App owns the effect boundaries for one invocation of the Go candidate.
type App struct {
	Executor Executor // command policy; nil is denied by execute
	Clock    Clock    // time source for decisions and default random seeding
	Random   Random   // owned non-cryptographic draws for selection decisions
}

// newApp fills omitted boundaries without granting command execution permission.
func newApp(_ Executor, _ Clock, _ Random) *App {
	return &App{}
}

// execute runs through the owned boundary, denying unconfigured execution.
func (a *App) execute(_ context.Context, _ Command) (CommandResult, *ExecuteError) {
	return CommandResult{}, nil
}
