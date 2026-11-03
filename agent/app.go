package main

import "context"

// App owns the effect boundaries for one invocation of the Go candidate.
type App struct {
	Executor Executor // command policy; nil is denied by execute
	Clock    Clock    // time source for decisions and default random seeding
	Random   Random   // owned non-cryptographic draws for selection decisions
}

// newApp fills omitted boundaries without granting command execution permission.
func newApp(executor Executor, clock Clock, random Random) *App {
	if executor == nil {
		executor = newDeniedExecutor()
	}
	if clock == nil {
		clock = newClock()
	}
	if random == nil {
		random = newRandom(clock.Now().UnixNano())
	}
	return &App{Executor: executor, Clock: clock, Random: random}
}

// execute runs through the owned boundary, denying unconfigured execution.
func (a *App) execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	if a.Executor == nil {
		return newDeniedExecutor().Execute(ctx, cmd)
	}
	return a.Executor.Execute(ctx, cmd)
}
