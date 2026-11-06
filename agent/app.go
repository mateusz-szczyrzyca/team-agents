package main

import (
	"context"
	"io"
)

// App owns the effect boundaries for one invocation of the Go candidate.
type App struct {
	Executor  Executor          // command policy; nil is denied by execute
	Clock     Clock             // time source for decisions and default random seeding
	Random    Random            // owned non-cryptographic draws for selection decisions
	tasksDir  string            // directory containing task metadata
	workTree  string            // git worktree directory for reports and checkpoints
	busDir    string            // agentbus state directory
	busBin    string            // path override for agentbus binary
	agentHome string            // agent home directory
	configEnv map[string]string // configuration-exported environment variables to propagate
	Stdout    io.Writer         // standard output destination for configuration replay
	Stderr    io.Writer         // standard error destination for configuration replay
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

// mergeConfigEnv returns configuration-exported environment variables for subprocess execution.
// Returns a new slice containing configuration exports that should be passed to subprocesses.
func (a *App) mergeConfigEnv() []string {
	if len(a.configEnv) == 0 {
		return nil
	}

	// Create result with configuration exports
	result := make([]string, 0, len(a.configEnv))
	for key, value := range a.configEnv {
		result = append(result, key+"="+value)
	}
	return result
}
