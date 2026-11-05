package main

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"strconv"
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
	config    *Config   // evaluated startup configuration; nil for explicitly constructed test apps
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

// loadStartupConfig prepares production paths and configuration before dispatch.
func (a *App) loadStartupConfig(ctx context.Context, args []string) *LoadConfigError {
	home := os.Getenv("HOME")
	agentsHome := os.Getenv("AGENTS_HOME")
	if agentsHome == "" {
		agentsHome = filepath.Join(home, "agents")
	}
	self, selfErr := os.Executable()
	if selfErr != nil {
		return fromProcess(LoadConfigShell, processErr("startup", selfErr, "locate executable"))
	}
	hostname := os.Getenv("HOSTNAME")
	if hostname == "" {
		detected, hostErr := os.Hostname()
		if hostErr != nil {
			return fromProcess(LoadConfigShell, processErr("startup", hostErr, "read hostname"))
		}
		hostname = detected
	}
	cfg, err := a.loadConfig(ctx, ConfigRequest{
		Args: args, File: filepath.Join(agentsHome, "agents.conf"),
		Context: ConfigContext{Home: home, AgentsHome: agentsHome, Hostname: hostname, UID: strconv.Itoa(os.Getuid()), Self: self},
	})
	if err != nil {
		return err
	}
	a.config = &cfg
	a.agentHome = cfg.Scalars["AGENTS_HOME"]
	a.tasksDir = cfg.Scalars["TASKS"]
	a.busDir = cfg.Scalars["BUS_DIR"]
	a.busBin = cfg.Scalars["AGENTBUS_BIN"]
	return nil
}

// listingConfig uses production settings or oracle capacities for injected test apps.
func (a *App) listingConfig() *Config {
	if a.config != nil {
		return a.config
	}
	return &Config{Scalars: map[string]string{
		"AGENTBUS_BIN": a.busBinary(), "INVESTIGATION_QUEUE_MAX": "3", "INV_SLOTS": "1",
	}}
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
