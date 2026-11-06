package main

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"syscall"
	"unsafe"
)

// App owns the effect boundaries for one invocation of the Go candidate.
type App struct {
	Executor  Executor  // command policy; nil is denied by execute
	Clock     Clock     // time source for decisions and default random seeding
	Random    Random    // owned non-cryptographic draws for selection decisions
	Stderr    io.Writer // standard error destination for diagnostics and subprocess output
	Stdout    io.Writer // ordinary configuration output destination; nil discards
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
		if home = os.Getenv("AGENTS_HOME"); home == "" {
			home = filepath.Join(os.Getenv("HOME"), "agents")
		}
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
	if err := os.Setenv("XDG_RUNTIME_DIR", a.xdgRuntimeDir()); err != nil {
		return fromProcess(LoadConfigShell, processErr("startup", err, "set XDG_RUNTIME_DIR"))
	}
	return nil
}

// xdgRuntimeDir returns the configured XDG_RUNTIME_DIR, environment override, or default /run/user/<uid>.
func (a *App) xdgRuntimeDir() string {
	if a != nil && a.config != nil {
		if v, ok := a.config.Scalars["XDG_RUNTIME_DIR"]; ok && v != "" {
			return v
		}
	}
	if v := os.Getenv("XDG_RUNTIME_DIR"); v != "" {
		return v
	}
	return "/run/user/" + strconv.Itoa(os.Getuid())
}

// newLogger returns a Logger configured with the application's resolved log directory and settings.
func (a *App) newLogger(subcommand string) *Logger {
	logDir := ""
	if a != nil && a.config != nil {
		logDir = a.config.Scalars["LOG_DIR"]
	}
	if logDir == "" {
		base := filepath.Join(os.Getenv("HOME"), "agents")
		if a != nil && a.agentHome != "" {
			base = a.agentHome
		}
		logDir = filepath.Join(base, "logs")
	}
	stderr := io.Writer(os.Stderr)
	var clock Clock
	if a != nil {
		if a.Stderr != nil {
			stderr = a.Stderr
		}
		clock = a.Clock
	}
	return &Logger{LogDir: logDir, Subcommand: subcommand, Stderr: stderr, Clock: clock, Journal: true, App: a, Interactive: isTerminal(stderr)}
}

// isTerminal reports whether w is connected to an interactive terminal descriptor.
func isTerminal(w io.Writer) bool {
	f, ok := w.(*os.File)
	if !ok {
		return false
	}
	var termios syscall.Termios
	_, _, err := syscall.Syscall6(syscall.SYS_IOCTL, f.Fd(), uintptr(syscall.TCGETS), uintptr(unsafe.Pointer(&termios)), 0, 0, 0) // #nosec G103 -- TCGETS ioctl determines terminal state
	return err == 0
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
	if a != nil && cmd.Env == nil {
		baseEnv := os.Environ()
		if a.config != nil && len(a.config.Env) > 0 {
			baseEnv = a.config.Env
		}
		cmd.Env = append([]string(nil), baseEnv...)
		if xdg := a.xdgRuntimeDir(); xdg != "" {
			cmd.Env = append(cmd.Env, "XDG_RUNTIME_DIR="+xdg)
		}
	}
	if a.Executor == nil {
		return newDeniedExecutor().Execute(ctx, cmd)
	}
	return a.Executor.Execute(ctx, cmd)
}
