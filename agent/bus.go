package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"strconv"
)

// ScriptNotice is the typed kind of an event published by the agent script on the bus.
type ScriptNotice string

const (
	NoticeMemberFinished     ScriptNotice = "member_finished"     // a team member finished (STATUS: DONE)
	NoticeMemberFailed       ScriptNotice = "member_failed"       // a team member's run failed
	NoticeMemberStopped      ScriptNotice = "member_stopped"      // the human stopped a team member
	NoticeMemberQuestions    ScriptNotice = "member_questions"    // a member's questions forwarded to the coordinator
	NoticeMemberNoStatus     ScriptNotice = "member_no_status"    // a member ended its turn without STATUS
	NoticeTeamStuck          ScriptNotice = "team_stuck"          // all members are asleep
	NoticeWorkerHung         ScriptNotice = "worker_hung"         // a worker was stopped by the watchdog
	NoticeAgentHung          ScriptNotice = "agent_hung"          // a normal member was marked hung
	NoticeCoordinatorHung    ScriptNotice = "coordinator_hung"    // the coordinator was marked hung
	NoticeHelperResult       ScriptNotice = "helper_result"       // a worker request finished (result to the requester)
	NoticeHelperStartFailure ScriptNotice = "helper_start_failed" // a worker request could not be started
	NoticeMemberPaused       ScriptNotice = "member_paused"       // a member is paused until a given time
	NoticeMemberBack         ScriptNotice = "member_back"         // a remote member is back after its node's restart
	NoticeMemberOut          ScriptNotice = "member_out"          // a member is out: items return to board
)

// Valid reports whether n is a known script notice.
func (n ScriptNotice) Valid() bool {
	switch n {
	case NoticeMemberFinished, NoticeMemberFailed, NoticeMemberStopped, NoticeMemberQuestions, NoticeMemberNoStatus,
		NoticeTeamStuck, NoticeWorkerHung, NoticeAgentHung, NoticeCoordinatorHung, NoticeHelperResult,
		NoticeHelperStartFailure, NoticeMemberPaused, NoticeMemberBack, NoticeMemberOut:
		return true
	default:
		return false
	}
}

// UnmarshalJSON validates the notice; errors contain *ValueError.
func (n *ScriptNotice) UnmarshalJSON(data []byte) error {
	var text string
	if err := json.Unmarshal(data, &text); err != nil {
		return valueWrap(err, "invalid script notice").asError()
	}
	candidate := ScriptNotice(text)
	if !candidate.Valid() {
		return valueErr("unknown script notice %q", text).asError()
	}
	*n = candidate
	return nil
}

// Exit codes from the agentbus CLI subcommands.
const (
	BusExitSuccess         = 0  // successful subcommand exit
	BusExitInvalidArgument = 2  // argument error (maps to BusInvalidArgument)
	BusExitNotFound        = 3  // not found error (maps to BusNotFound)
	BusExitNotInTeam       = 4  // not in team error (maps to BusNotInTeam)
	BusExitPermission      = 5  // permission error (maps to BusPermission)
	BusExitConflict        = 6  // conflict or lock error (maps to BusConflict)
	BusExitInvalidState    = 7  // invalid state transition (maps to BusInvalidState)
	BusExitStorage         = 8  // storage or I/O failure (maps to BusStorage)
	BusExitUnknownTool     = 9  // unknown tool requested (maps to BusUnknownTool)
	BusExitProcess         = 10 // subprocess helper failure (maps to BusProcess)
)

// BusServeParams defines arguments used to build the agentbus serve CLI invocation.
type BusServeParams struct {
	BusDir     string // state directory for the bus
	Agent      string // name of the agent running the session
	TaskID     string // identifier of the task
	TaskDir    string // directory holding task files and metadata
	RepoName   string // name of the repository
	Branch     string // git branch of the task
	Worktree   string // path to the task worktree
	Peers      string // comma-separated list of peer agent names
	Role       string // role of the agent: worker, coordinator, or investigator
	QueueMax   int    // maximum capacity for investigation queues
	InvSlots   int    // allocated investigation slots
	TasksDir   string // directory holding all tasks
	Team       string // optional team identifier
	Tier       string // agent tier (e.g. normal or worker)
	AgentCmd   string // path to the agent binary itself
	AgentsHome string // path to the AGENTS_HOME directory
	RemoteTeam bool   // whether this is a remote team member
	JobLead    string // lead URL or address for remote team relay
	Member     string // member name for remote team relay
}

// mapBusExitCode converts an agentbus process exit code and stderr to a typed BusError.
func mapBusExitCode(code int, stderr string) *BusError {
	if code == BusExitSuccess {
		return nil
	}
	cause := BusStorage
	switch code {
	case BusExitInvalidArgument:
		cause = BusInvalidArgument
	case BusExitNotFound:
		cause = BusNotFound
	case BusExitNotInTeam:
		cause = BusNotInTeam
	case BusExitPermission:
		cause = BusPermission
	case BusExitConflict:
		cause = BusConflict
	case BusExitInvalidState:
		cause = BusInvalidState
	case BusExitStorage:
		cause = BusStorage
	case BusExitUnknownTool:
		cause = BusUnknownTool
	case BusExitProcess:
		cause = BusProcess
	}
	return opErr(cause, "%s", stderr)
}

// busArgsJSON constructs the argv list for agentbus serve matching bash bus_args_json.
func busArgsJSON(params BusServeParams) []string {
	args := []string{"serve"}

	if params.RemoteTeam {
		if params.JobLead != "" {
			args = append(args, "--relay", params.JobLead)
		}
		if params.Member != "" {
			args = append(args, "--member", params.Member)
		}
	} else {
		if params.BusDir != "" {
			args = append(args, "--dir", params.BusDir)
		}
		if params.Agent != "" {
			args = append(args, "--agent", params.Agent)
		}
		if params.TaskID != "" {
			args = append(args, "--task", params.TaskID)
		}
		if params.TaskDir != "" {
			args = append(args, "--task-dir", params.TaskDir)
		}
		if params.RepoName != "" {
			args = append(args, "--repo", params.RepoName)
		}
		if params.Branch != "" {
			args = append(args, "--branch", params.Branch)
		}
		if params.Worktree != "" {
			args = append(args, "--worktree", params.Worktree)
		}
		if params.Peers != "" {
			args = append(args, "--peers", params.Peers)
		}
		if params.Role != "" {
			args = append(args, "--role", params.Role)
		}
		args = append(args, "--queue-max", strconv.Itoa(params.QueueMax))
		args = append(args, "--inv-slots", strconv.Itoa(params.InvSlots))
		if params.TasksDir != "" {
			args = append(args, "--tasks-dir", params.TasksDir)
		}
		if params.Team != "" {
			args = append(args, "--team", params.Team)
		}
		if params.Tier != "" {
			args = append(args, "--tier", params.Tier)
		}
		if params.AgentCmd != "" {
			args = append(args, "--agent-cmd", params.AgentCmd)
		}
	}

	if params.AgentsHome != "" {
		args = append(args, "--home", params.AgentsHome)
	}

	return args
}

// busMessagesFrom counts messages matching from: agent in messages.jsonl.
func busMessagesFrom(messagesPath string, agent string) (count int, err *StorageError) {
	file, openErr := os.Open(messagesPath) // #nosec G304 -- path to messages.jsonl is constructed by the agent runner
	if openErr != nil {
		if errors.Is(openErr, os.ErrNotExist) {
			return 0, nil
		}
		return 0, storageErr(openErr, "open messages file %s", messagesPath)
	}
	defer func() {
		if closeErr := file.Close(); closeErr != nil {
			cerr := storageErr(closeErr, "close messages file %s", messagesPath)
			if err == nil {
				err = cerr
			} else {
				err.Err = errors.Join(err.Err, closeErr)
			}
		}
	}()

	reader := bufio.NewReader(file)
	for {
		line, readErr := reader.ReadBytes('\n')
		if len(line) > 0 {
			var msg struct {
				From string `json:"from"`
			}
			if json.Unmarshal(line, &msg) == nil && msg.From == agent {
				count++
			}
		}
		if readErr != nil {
			if errors.Is(readErr, io.EOF) {
				break
			}
			err = storageErr(readErr, "read messages file %s", messagesPath)
			break
		}
	}
	return count, err
}

// busCmd executes an agentbus subcommand via the app executor and maps its status code.
func busCmd(ctx context.Context, app *App, busBin string, args ...string) (string, *BusError) {
	var stdout, stderr bytes.Buffer
	cmd := Command{
		Name:   busBin,
		Args:   args,
		Stdout: &stdout,
		Stderr: &stderr,
		Env:    app.mergeConfigEnv(),
	}
	result, eerr := app.execute(ctx, cmd)
	if eerr != nil {
		return "", opErr(BusProcess, "execute %s: %s", busBin, eerr.Error())
	}
	if result.ExitCode != BusExitSuccess {
		// Preserve OS exit codes 126 (permission denied) and 127 (not found) as process errors.
		if result.ExitCode == 126 || result.ExitCode == 127 {
			return "", opErr(BusProcess, "execute %s: exit code %d", busBin, result.ExitCode)
		}
		return "", mapBusExitCode(result.ExitCode, stderr.String())
	}
	return stdout.String(), nil
}

// sendNotice constructs and delivers a typed script notice on the bus, logging delivery and warning on failure.
func sendNotice(ctx context.Context, app *App, logger *Logger, busBin, busDir string, kind ScriptNotice, from, to, text string, fields ...string) *LogWriteError {
	args := []string{"send", "--dir", busDir, "--from", from, "--to", to, "--notice", string(kind)}
	for _, field := range fields {
		args = append(args, "--field", field)
	}
	args = append(args, text)
	if err := Try(ctx, app, logger, LogNoticeFailed, Command{Name: busBin, Args: args, Env: app.mergeConfigEnv()}); err != nil {
		return err
	}
	if logger != nil {
		return logger.Log(ctx, LevelInfo, LogNoticeSent, "notice="+string(kind), "to="+to)
	}
	return nil
}
