package main

import (
	"context"
	"strings"
)

// App owns the effect boundaries for one invocation of the Go candidate.
type App struct {
	Executor  Executor           // command policy; nil is denied by execute
	Clock     Clock              // time source for decisions and default random seeding
	Random    Random             // owned non-cryptographic draws for selection decisions
	tasksDir  string             // directory containing task metadata
	workTree  string             // git worktree directory for reports and checkpoints
	busDir    string             // agentbus state directory
	busBin    string             // path override for agentbus binary
	agentHome string             // agent home directory
	configEnv map[string]string // configuration-exported environment variables to propagate
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

// mergeConfigEnv merges configuration-exported environment variables with a base environment.
// If baseEnv is nil, a new slice is created with configuration exports only.
// If baseEnv is not nil, configuration exports are appended to it, overriding any existing values.
func (a *App) mergeConfigEnv(baseEnv []string) []string {
	if a.configEnv == nil || len(a.configEnv) == 0 {
		return baseEnv
	}

	// Create a map to track which env vars we've seen to avoid duplicates
	seen := make(map[string]bool)
	result := make([]string, 0, len(baseEnv)+len(a.configEnv))

	// Add base environment, tracking seen keys
	if baseEnv != nil {
		for _, entry := range baseEnv {
			parts := strings.SplitN(entry, "=", 2)
			if len(parts) > 0 {
				seen[parts[0]] = true
			}
			result = append(result, entry)
		}
	}

	// Add configuration exports, replacing any that were in base
	for key, value := range a.configEnv {
		if seen[key] {
			// Remove the old value
			filtered := make([]string, 0, len(result))
			for _, entry := range result {
				if !strings.HasPrefix(entry, key+"=") {
					filtered = append(filtered, entry)
				}
			}
			result = filtered
		}
		result = append(result, key+"="+value)
	}

	return result
}
