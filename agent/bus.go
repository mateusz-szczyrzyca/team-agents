package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

// ScriptNotice represents a typed bus notice.
type ScriptNotice string

// ScriptNotice constants.
const (
	NoticeMemberFinished    ScriptNotice = "member_finished"
	NoticeMemberFailed      ScriptNotice = "member_failed"
	NoticeMemberStopped     ScriptNotice = "member_stopped"
	NoticeMemberQuestions   ScriptNotice = "member_questions"
	NoticeMemberNoStatus    ScriptNotice = "member_no_status"
	NoticeTeamStuck         ScriptNotice = "team_stuck"
	NoticeWorkerHung        ScriptNotice = "worker_hung"
	NoticeAgentHung         ScriptNotice = "agent_hung"
	NoticeCoordinatorHung   ScriptNotice = "coordinator_hung"
	NoticeHelperResult      ScriptNotice = "helper_result"
	NoticeHelperStartFailure ScriptNotice = "helper_start_failed"
	NoticeMemberPaused      ScriptNotice = "member_paused"
	NoticeMemberBack        ScriptNotice = "member_back"
	NoticeMemberOut         ScriptNotice = "member_out"
)

// Valid reports whether n is a valid ScriptNotice.
func (n ScriptNotice) Valid() bool {
	switch n {
	case NoticeMemberFinished, NoticeMemberFailed, NoticeMemberStopped,
		NoticeMemberQuestions, NoticeMemberNoStatus, NoticeTeamStuck,
		NoticeWorkerHung, NoticeAgentHung, NoticeCoordinatorHung,
		NoticeHelperResult, NoticeHelperStartFailure, NoticeMemberPaused,
		NoticeMemberBack, NoticeMemberOut:
		return true
	}
	return false
}

// UnmarshalJSON validates the notice during JSON decoding.
func (n *ScriptNotice) UnmarshalJSON(data []byte) error {
	var str string
	if err := json.Unmarshal(data, &str); err != nil {
		return err
	}
	notice := ScriptNotice(str)
	if !notice.Valid() {
		return fmt.Errorf("invalid notice: %s", str)
	}
	*n = notice
	return nil
}

// BusCause is an error cause for bus operations.
type BusCause string

// BusCause constants.
const (
	BusInvalidArgument BusCause = "InvalidArgument"
	BusNotFound        BusCause = "NotFound"
	BusNotInTeam       BusCause = "NotInTeam"
	BusPermission      BusCause = "Permission"
	BusConflict        BusCause = "Conflict"
	BusInvalidState    BusCause = "InvalidState"
	BusStorage         BusCause = "Storage"
	BusUnknownTool     BusCause = "UnknownTool"
	BusProcess         BusCause = "Process"
)

// BusError is an error from agentbus.
type BusError struct {
	Cause BusCause // why the operation failed
	Text  string   // what went wrong
	Err   error    // the class sentinel for errors.Is
}

// Error implements the error interface.
func (e *BusError) Error() string {
	return e.Text
}

// Unwrap exposes the class sentinel for errors.Is.
func (e *BusError) Unwrap() error {
	return e.Err
}

// Class returns the ErrorClass for this error.
func (e *BusError) Class() ErrorClass {
	switch e.Cause {
	case BusInvalidArgument:
		return ClassInvalidArgument
	case BusNotFound:
		return ClassNotFound
	case BusNotInTeam:
		return ClassNotInTeam
	case BusPermission:
		return ClassPermission
	case BusConflict:
		return ClassConflict
	case BusInvalidState:
		return ClassInvalidState
	case BusStorage:
		return ClassStorage
	case BusUnknownTool:
		return ClassUnknownTool
	case BusProcess:
		return ClassProcess
	default:
		return ClassStorage // Default to storage error
	}
}

// Bus exit codes matching agentbus/errors.go.
const (
	BusExitSuccess          = 0
	BusExitInvalidArgument  = 2
	BusExitNotFound         = 3
	BusExitNotInTeam        = 4
	BusExitPermission       = 5
	BusExitConflict         = 6
	BusExitInvalidState     = 7
	BusExitStorage          = 8
	BusExitUnknownTool      = 9
	BusExitProcess          = 10
)

// mapBusExitCode converts an agentbus exit code to a BusError.
func mapBusExitCode(code int, text string) *BusError {
	if code == BusExitSuccess {
		return nil
	}

	cause := BusStorage // Default to storage error for unknown codes
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

	// Create error with class sentinel
	busErr := &BusError{
		Cause: cause,
		Text:  text,
	}

	// Create a temporary error to get the class and sentinel
	class := busErr.Class()
	busErr.Err = class.Sentinel()

	return busErr
}

// BusServeParams contains parameters for agentbus serve.
type BusServeParams struct {
	// Local mode
	BusDir     string
	Agent      string
	TaskID     string
	TaskDir    string
	RepoName   string
	Branch     string
	Worktree   string
	Peers      string
	Role       string
	QueueMax   int
	InvSlots   int
	TasksDir   string
	Team       string
	Tier       string
	AgentCmd   string
	AgentsHome string

	// Remote mode
	RemoteTeam bool
	JobLead    string
	Member     string
}

// busArgsJSON builds argv for agentbus serve.
func busArgsJSON(params BusServeParams) []string {
	var args []string
	args = append(args, "serve")

	if params.RemoteTeam {
		// Remote mode
		if params.JobLead != "" {
			args = append(args, "--relay", params.JobLead)
		}
		if params.Member != "" {
			args = append(args, "--member", params.Member)
		}
	} else {
		// Local mode
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
		args = append(args, "--queue-max", fmt.Sprintf("%d", params.QueueMax))
		args = append(args, "--inv-slots", fmt.Sprintf("%d", params.InvSlots))
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

// busMessagesFrom counts messages from a specific agent in messages.jsonl.
func busMessagesFrom(path, agent string) (int, error) {
	file, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return 0, nil // File doesn't exist yet, count is 0
		}
		return 0, storageErr(err, "open messages.jsonl")
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	const maxBufferSize = 1024 * 1024
	scanner.Buffer(make([]byte, bufio.MaxScanTokenSize), maxBufferSize)
	count := 0

	for scanner.Scan() {
		var msg map[string]interface{}
		if err := json.Unmarshal(scanner.Bytes(), &msg); err != nil {
			continue // Skip malformed lines
		}

		if from, ok := msg["from"].(string); ok && from == agent {
			count++
		}
	}

	if err := scanner.Err(); err != nil {
		return 0, storageErr(err, "scan messages.jsonl")
	}

	return count, nil
}
