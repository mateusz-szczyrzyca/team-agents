package main

import (
	"context"
	"io"
	"os"
	"path/filepath"
)

// App owns the effect boundaries for one invocation of the Go candidate.
type App struct {
	Executor  Executor  // command policy; nil is denied by execute
	Clock     Clock     // time source for decisions and default random seeding
	Random    Random    // owned non-cryptographic draws for selection decisions
	Stderr    io.Writer // standard error destination for diagnostics and subprocess output
	tasksDir  string    // directory containing task metadata
	workTree  string    // git worktree directory for reports and checkpoints
	busDir    string    // agentbus state directory
	busBin    string    // path override for agentbus binary
	agentHome string    // agent home directory
}

// tasksDirectory returns the configured tasks directory or resolves it from agent home.
func (a *App) tasksDirectory() string {
	if a.tasksDir != "" {
		return a.tasksDir
	}
	home := a.agentHome
	if home == "" {
		home = os.Getenv("AGENTS_HOME")
	}
	if home == "" {
		home = filepath.Join(os.Getenv("HOME"), "agents")
	}
	return filepath.Join(home, "tasks")
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
