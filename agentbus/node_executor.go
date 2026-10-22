// The checked node executor invokes only fixed operator-installed script bridges.
package main

import (
	"bufio"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// nodeChild identifies a process group created by this daemon, never a PID read from an untrusted file.
type nodeChild struct {
	command *exec.Cmd     // owned child; Wait is called exactly once
	done    chan struct{} // closed after Wait and log cleanup
}

// nodeProcessExecutor owns running child groups; its executable and home come only from local configuration.
type nodeProcessExecutor struct {
	Home         string                // trusted local task root
	AgentCmd     string                // fixed bridge executable
	Instance     string                // local qualified-name suffix
	Capabilities []Capability          // permitted declared capabilities
	AcceptRepos  []string              // optional extra allowlist consumed by local bridge validation
	mu           sync.Mutex            // protects the process map
	children     map[string]*nodeChild // current daemon's process groups by task
	finished     map[string]int        // PIDs whose Wait completed in this daemon, fencing stale pidfile reuse
}

// nodeCurrentState asks the fixed local bridge to serialize already-sourced Bash configuration and metadata.
func nodeCurrentState(program, home string) (InstanceCurrentState, *ProcessError) {
	var current InstanceCurrentState
	if program == "" || !filepath.IsAbs(program) {
		return current, processErr(program, nil, "node requires an absolute agent script path")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, program, "_node-manifest") //nolint:gosec // G204: executable is operator configuration and the bridge name is fixed
	cmd.Env = append(os.Environ(), "AGENTS_HOME="+home)
	out, err := cmd.Output()
	if err != nil {
		return current, processErr(program, err, "cannot collect local node manifest")
	}
	if len(out) > hubBodyLimit {
		return current, processErr(program, nil, "local manifest exceeds the size limit")
	}
	if err := json.Unmarshal(out, &current); err != nil {
		return current, processErr(program, err, "cannot decode local node manifest")
	}
	return current, nil
}

// taskPath rejects traversal and symlink escapes before the bridge receives a task identifier.
func (e *nodeProcessExecutor) taskPath(task string) (string, *ProcessError) {
	if !validNodeTaskID(task) {
		return "", processErr(e.AgentCmd, nil, "invalid task identifier")
	}
	root, err := filepath.EvalSymlinks(filepath.Join(e.Home, "tasks"))
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", processErr(e.AgentCmd, err, "cannot inspect local task root")
	}
	path, err := filepath.EvalSymlinks(filepath.Join(root, task))
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", processErr(e.AgentCmd, err, "cannot inspect local task")
	}
	relative, err := filepath.Rel(root, path)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
		return "", processErr(e.AgentCmd, nil, "task path escapes the local task root")
	}
	info, err := os.Stat(filepath.Join(path, "meta.env"))
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", processErr(e.AgentCmd, err, "cannot inspect local task metadata")
	}
	if !info.Mode().IsRegular() {
		return "", processErr(e.AgentCmd, nil, "local task metadata must be a regular file")
	}
	return path, nil
}

// checkRequirements enforces declared local policy and explicit approvals for sensitive work.
func (e *nodeProcessExecutor) checkRequirements(command Command) *ProcessError {
	var required []Capability
	for _, raw := range strings.FieldsFunc(command.Params["requires"], func(c rune) bool { return c == ',' || c == ' ' }) {
		capability, err := ParseCapability(raw)
		if err != nil {
			return processErr(e.AgentCmd, nil, "unknown required capability %q", raw)
		}
		permitted := false
		for _, local := range e.Capabilities {
			if capability == local {
				permitted = true
				break
			}
		}
		if !permitted {
			return processErr(e.AgentCmd, nil, "required capability %q is not declared locally", capability)
		}
		if capability.IsSensitive() {
			required = append(required, capability)
		}
	}
	if len(required) == 0 {
		return nil
	}
	data, err := os.ReadFile(filepath.Join(e.Home, "node", "approved-capabilities.json"))
	if errors.Is(err, os.ErrNotExist) {
		return processErr(e.AgentCmd, nil, "sensitive capabilities require local human approval before execution")
	}
	if err != nil {
		return processErr(e.AgentCmd, err, "cannot read local capability approvals")
	}
	var approved []Capability
	if err := json.Unmarshal(data, &approved); err != nil {
		return processErr(e.AgentCmd, err, "cannot decode local capability approvals")
	}
	for _, required := range required {
		allowed := false
		for _, capability := range approved {
			if capability == required && capability.Valid() {
				allowed = true
				break
			}
		}
		if !allowed {
			return processErr(e.AgentCmd, nil, "capability %q requires local human approval", required)
		}
	}
	return nil
}

// Execute returns an explicit unknown outcome after an interrupted acceptance instead of spawning twice.
func (e *nodeProcessExecutor) Execute(ctx context.Context, command Command, uncertain bool) (NodeExecutionResult, *ProcessError) {
	if !command.Kind.Valid() {
		return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "unknown node command kind")
	}
	e.mu.Lock()
	child := e.children[command.Task]
	e.mu.Unlock()
	if uncertain {
		if child != nil && (command.Kind == CommandStartRun || command.Kind == CommandWake) {
			return NodeExecutionResult{Outcome: CommandOutcomeAlreadyRunning}, nil
		}
		return NodeExecutionResult{Outcome: CommandOutcomeFailed, Detail: "outcome unknown after node restart; inspect the task before submitting replacement work"}, nil
	}
	if err := e.checkRequirements(command); err != nil {
		return NodeExecutionResult{}, err
	}
	path, err := e.taskPath(command.Task)
	if err != nil {
		return NodeExecutionResult{}, err
	}
	if path == "" {
		if command.Kind == CommandStartRun && command.Params["repo"] != "" {
			provisionArgs := []string{
				"_node-provision",
				command.Task,
				command.Params["agent"],
				command.Params["repo"],
				command.Params["branch"],
				command.Params["base"],
				command.Params["team"],
			}
			_, pErr := e.runBridge(ctx, CommandOutcomeStarted, provisionArgs...)
			if pErr != nil {
				return NodeExecutionResult{Outcome: CommandOutcomeFailed, Detail: pErr.Message}, pErr
			}
			path, err = e.taskPath(command.Task)
			if err != nil || path == "" {
				return NodeExecutionResult{Outcome: CommandOutcomeUnknownTask}, nil
			}
		} else {
			return NodeExecutionResult{Outcome: CommandOutcomeUnknownTask}, nil
		}
	}

	switch command.Kind {
	case CommandStartRun:
		run, err := strconv.Atoi(command.Params["run"])
		if err != nil || run <= 0 || !validNodeTaskID(command.Params["agent"]) {
			return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "start_run requires a valid local agent and positive run")
		}
		current, manifestErr := nodeCurrentState(e.AgentCmd, e.Home)
		if manifestErr != nil {
			return NodeExecutionResult{}, manifestErr
		}
		if task, ok := current.Tasks[command.Task]; !ok || task.Agent != command.Params["agent"] {
			return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "start_run agent does not match locally provisioned task")
		}

		for taskID, other := range current.Tasks {
			if taskID != command.Task && other.Agent == command.Params["agent"] && nodeTaskReservesAgent(other.Status) {
				return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "agent is reserved by another local task")
			}
		}
		if child == nil {
			if safe, err := e.priorRunnerFinished(path, command.Task); err != nil {
				return NodeExecutionResult{}, err
			} else if !safe {
				return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "task has an unowned prior runner identity; reconcile it before starting another run")
			}
		}
		mode := "new"
		if run > 1 {
			mode = "resume"
		}
		return e.launch(ctx, command.Task, CommandOutcomeStarted, "_node-start", command.Task, mode, strconv.Itoa(run))
	case CommandWake:
		return e.launch(ctx, command.Task, CommandOutcomeDelivered, "_node-wake", command.Task)
	case CommandStopTask:
		if child == nil {
			if safe, err := e.priorRunnerFinished(path, command.Task); err != nil {
				return NodeExecutionResult{}, err
			} else if !safe {
				return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "task runner is not owned by this daemon; reconcile its identity before stopping it")
			}
		}
		if err := e.stop(command.Task); err != nil {
			return NodeExecutionResult{}, err
		}
		// The bridge updates task metadata; process termination uses only groups owned by this daemon.
		return e.runBridge(ctx, CommandOutcomeStopped, "_node-mark-stopped", command.Task)
	case CommandAnswerDeliver:
		answer := command.Params["answer"]
		round, roundErr := strconv.Atoi(command.Params["question_round"])
		snapshot := command.Params["snapshot_version"]
		if roundErr != nil || round <= 0 || len(snapshot) != 64 {
			return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "answer_deliver requires the question round and SHA-256 snapshot version")
		}
		if _, err := hex.DecodeString(snapshot); err != nil {
			return NodeExecutionResult{}, processErr(e.AgentCmd, err, "invalid question snapshot version")
		}
		if answer == "" || len(answer) > hubBodyLimit {
			return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "answer_deliver requires a bounded answer payload")
		}
		if child != nil {
			return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "cannot deliver answers to a task with a running child")
		}
		return e.launchWithInput(ctx, command.Task, CommandOutcomeDelivered, answer, "_node-answer", command.Task, strconv.Itoa(round), snapshot)
	default:
		return NodeExecutionResult{}, processErr(e.AgentCmd, nil, "unsupported node command")
	}
}

// launch starts a supervised bridge without sending arguments through a shell.
func (e *nodeProcessExecutor) launch(ctx context.Context, task string, outcome CommandOutcome, args ...string) (NodeExecutionResult, *ProcessError) {
	return e.launchWithInput(ctx, task, outcome, "", args...)
}

// launchWithInput waits for durable script admission before reporting a successful command outcome.
func (e *nodeProcessExecutor) launchWithInput(ctx context.Context, task string, outcome CommandOutcome, input string, args ...string) (NodeExecutionResult, *ProcessError) {
	child, read, err := e.startProcess(task, input, args...)
	if err != nil {
		return NodeExecutionResult{}, err
	}
	if child == nil {
		return NodeExecutionResult{Outcome: CommandOutcomeAlreadyRunning}, nil
	}
	defer closeHubFile(read)
	// The buffered completion channel lets closing read cancel the observer without leaking a goroutine.
	type readiness struct {
		line   string        // bounded private bridge signal
		failed *ProcessError // a closed or malformed readiness pipe
	}
	ready := make(chan readiness, 1)
	go func() {
		line, err := bufio.NewReader(io.LimitReader(read, 128)).ReadString('\n')
		var failed *ProcessError
		if err != nil {
			failed = processErr(e.AgentCmd, err, "node bridge did not confirm admission")
		}
		ready <- readiness{line: line, failed: failed}
	}()
	timer := time.NewTimer(10 * time.Second)
	defer timer.Stop()
	var signal readiness
	select {
	case signal = <-ready:
	case <-ctx.Done():
		signal.failed = processErr(e.AgentCmd, ctx.Err(), "node bridge admission canceled")
	case <-timer.C:
		signal.failed = processErr(e.AgentCmd, nil, "node bridge admission timed out")
	}
	if signal.failed == nil {
		switch CommandOutcome(strings.TrimSpace(signal.line)) {
		case CommandOutcomeStarted, CommandOutcomeDelivered:
			return NodeExecutionResult{Outcome: outcome}, nil
		case CommandOutcomeAlreadyRunning:
			return NodeExecutionResult{Outcome: CommandOutcomeAlreadyRunning}, nil
		default:
			signal.failed = processErr(e.AgentCmd, nil, "node bridge returned invalid readiness")
		}
	}
	if err := e.stop(task); err != nil {
		warnSideEffect("stop rejected node bridge", err)
	}
	return NodeExecutionResult{}, signal.failed
}

// startProcess installs a child process group and a private readiness pipe before returning to the journal caller.
func (e *nodeProcessExecutor) startProcess(task, input string, args ...string) (*nodeChild, *os.File, *ProcessError) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.children[task] != nil {
		return nil, nil, nil
	}
	if !filepath.IsAbs(e.AgentCmd) {
		return nil, nil, processErr(e.AgentCmd, nil, "agent script path must be absolute")
	}
	log, err := os.OpenFile(filepath.Join(e.Home, "node", "executor.log"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		return nil, nil, processErr(e.AgentCmd, err, "cannot open node executor log")
	}
	read, write, err := os.Pipe()
	if err != nil {
		closeHubFile(log)
		return nil, nil, processErr(e.AgentCmd, err, "cannot create node readiness pipe")
	}
	cmd := exec.Command(e.AgentCmd, args...) //nolint:gosec // G204: fixed configured executable and allowlisted bridge arguments, never a hub-selected program
	cmd.Env = append(os.Environ(), "AGENTS_HOME="+e.Home, "NODE_ACCEPT_REPOS="+strings.Join(e.AcceptRepos, ","), "NODE_READY_FD=3")
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.ExtraFiles = []*os.File{write}
	cmd.Stdin = strings.NewReader(input)
	cmd.Stdout, cmd.Stderr = log, log
	if err := cmd.Start(); err != nil {
		closeHubFile(log)
		closeHubFile(read)
		closeHubFile(write)
		return nil, nil, processErr(e.AgentCmd, err, "cannot start node bridge")
	}
	closeHubFile(write)
	if e.children == nil {
		e.children = make(map[string]*nodeChild)
	}
	child := &nodeChild{command: cmd, done: make(chan struct{})}
	e.children[task] = child
	go func() {
		if err := cmd.Wait(); err != nil {
			warnSideEffect("node runner exit", processErr(e.AgentCmd, err, "node bridge exited unsuccessfully"), "task", task)
		}
		closeHubFile(log)
		e.mu.Lock()
		delete(e.children, task)
		if e.finished == nil {
			e.finished = make(map[string]int)
		}
		e.finished[task] = cmd.Process.Pid
		close(child.done)
		e.mu.Unlock()
	}()
	return child, read, nil
}

// runBridge invokes a short metadata operation and checks its completion.
func (e *nodeProcessExecutor) runBridge(ctx context.Context, outcome CommandOutcome, args ...string) (NodeExecutionResult, *ProcessError) {
	bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(bounded, e.AgentCmd, args...) //nolint:gosec // G204: operator executable and fixed private bridge operation
	cmd.Env = append(os.Environ(), "AGENTS_HOME="+e.Home, "NODE_ACCEPT_REPOS="+strings.Join(e.AcceptRepos, ","))
	cmd.Stdout, cmd.Stderr = io.Discard, io.Discard
	if err := cmd.Run(); err != nil {
		return NodeExecutionResult{}, processErr(e.AgentCmd, err, "node metadata bridge failed")
	}
	return NodeExecutionResult{Outcome: outcome}, nil
}

// stop signals only a process group this daemon created and waits outside the process-map lock.
func (e *nodeProcessExecutor) stop(task string) *ProcessError {
	e.mu.Lock()
	child := e.children[task]
	if child == nil {
		e.mu.Unlock()
		return nil
	}
	err := syscall.Kill(-child.command.Process.Pid, syscall.SIGTERM)
	e.mu.Unlock()
	if err != nil && !errors.Is(err, syscall.ESRCH) {
		return processErr(e.AgentCmd, err, "cannot stop owned node runner")
	}
	timer := time.NewTimer(time.Second)
	defer timer.Stop()
	select {
	case <-child.done:
		return nil
	case <-timer.C:
	}
	e.mu.Lock()
	// Wait may have completed while the grace timer fired; never signal a reused process group.
	if e.children[task] == child {
		err = syscall.Kill(-child.command.Process.Pid, syscall.SIGKILL)
	} else {
		err = nil
	}
	e.mu.Unlock()
	if err != nil && !errors.Is(err, syscall.ESRCH) {
		return processErr(e.AgentCmd, err, "cannot kill owned node runner")
	}
	<-child.done
	return nil
}

// Close tears down supervised groups; hub outages never call it because the daemon remains running.
func (e *nodeProcessExecutor) Close() *ProcessError {
	e.mu.Lock()
	tasks := make([]string, 0, len(e.children))
	for task := range e.children {
		tasks = append(tasks, task)
	}
	e.mu.Unlock()
	for _, task := range tasks {
		if err := e.stop(task); err != nil {
			return err
		}
	}
	return nil
}

// priorRunnerFinished accepts only an absent pidfile or a PID whose exact child Wait this daemon observed.
func (e *nodeProcessExecutor) priorRunnerFinished(path, task string) (bool, *ProcessError) {
	data, err := os.ReadFile(filepath.Join(path, "runner.pid")) //nolint:gosec // G304: path was constrained to the local task root by taskPath; the filename is fixed
	if errors.Is(err, os.ErrNotExist) {
		return true, nil
	}
	if err != nil {
		return false, processErr(e.AgentCmd, err, "cannot inspect task runner identity")
	}
	pid, err := strconv.Atoi(strings.TrimSpace(string(data)))
	if err != nil || pid <= 0 {
		return false, processErr(e.AgentCmd, nil, "task has invalid prior runner identity")
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.finished[task] == pid, nil
}

const (
	nodeTaskRunning  string = "running"  // an active CLI reserves its configured agent
	nodeTaskStarting string = "starting" // admission is complete but the runner is still starting
	nodeTaskSleeping string = "sleeping" // a sleeping team member retains its configured agent
	nodeTaskWaiting  string = "waiting"  // human questions retain a resumable registered run
	nodeTaskPaused   string = "paused"   // quota or wake-loop pause retains the agent's current task
)

// nodeTaskReservesAgent prevents one configured agent from executing concurrent task sessions.
func nodeTaskReservesAgent(status string) bool {
	return status == nodeTaskRunning || status == nodeTaskStarting || status == nodeTaskSleeping || status == nodeTaskPaused
}
