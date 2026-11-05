package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

// ScriptNotice represents a typed bus notice.
type ScriptNotice string

// ScriptNotice constants for bus message types.
const (
	// NoticeMemberFinished is sent when a team member completes its work.
	NoticeMemberFinished ScriptNotice = "member_finished"
	// NoticeMemberFailed is sent when a team member fails its work.
	NoticeMemberFailed ScriptNotice = "member_failed"
	// NoticeMemberStopped is sent when a team member is stopped or terminated.
	NoticeMemberStopped ScriptNotice = "member_stopped"
	// NoticeMemberQuestions is sent when a team member has questions for the human.
	NoticeMemberQuestions ScriptNotice = "member_questions"
	// NoticeMemberNoStatus is sent when a team member ends without status.
	NoticeMemberNoStatus ScriptNotice = "member_no_status"
	// NoticeTeamStuck is sent when a team becomes stuck or unresponsive.
	NoticeTeamStuck ScriptNotice = "team_stuck"
	// NoticeWorkerHung is sent when a worker agent hangs or stops responding.
	NoticeWorkerHung ScriptNotice = "worker_hung"
	// NoticeAgentHung is sent when an agent hangs or stops responding.
	NoticeAgentHung ScriptNotice = "agent_hung"
	// NoticeCoordinatorHung is sent when a coordinator hangs or stops responding.
	NoticeCoordinatorHung ScriptNotice = "coordinator_hung"
	// NoticeHelperResult is sent when a helper provides results.
	NoticeHelperResult ScriptNotice = "helper_result"
	// NoticeHelperStartFailure is sent when a helper fails to start.
	NoticeHelperStartFailure ScriptNotice = "helper_start_failed"
	// NoticeMemberPaused is sent when a member is paused.
	NoticeMemberPaused ScriptNotice = "member_paused"
	// NoticeMemberBack is sent when a paused member returns.
	NoticeMemberBack ScriptNotice = "member_back"
	// NoticeMemberOut is sent when a member is excluded from the team.
	NoticeMemberOut ScriptNotice = "member_out"
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

// BusCause constants represent error causes from bus operations.
const (
	// BusInvalidArgument indicates an invalid argument was provided to agentbus.
	BusInvalidArgument BusCause = "InvalidArgument"
	// BusNotFound indicates a requested resource was not found.
	BusNotFound BusCause = "NotFound"
	// BusNotInTeam indicates an agent is not in a team.
	BusNotInTeam BusCause = "NotInTeam"
	// BusPermission indicates insufficient permissions for the operation.
	BusPermission BusCause = "Permission"
	// BusConflict indicates a state conflict preventing the operation.
	BusConflict BusCause = "Conflict"
	// BusInvalidState indicates an invalid state for the operation.
	BusInvalidState BusCause = "InvalidState"
	// BusStorage indicates a storage or I/O error.
	BusStorage BusCause = "Storage"
	// BusUnknownTool indicates an unknown agentbus tool was called.
	BusUnknownTool BusCause = "UnknownTool"
	// BusProcess indicates a subprocess or process error.
	BusProcess BusCause = "Process"
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

// Bus exit code constants match agentbus/errors.go exit status codes.
const (
	// BusExitSuccess is the exit code for successful agentbus operations.
	BusExitSuccess = 0
	// BusExitInvalidArgument is the exit code for invalid arguments.
	BusExitInvalidArgument = 2
	// BusExitNotFound is the exit code when a resource is not found.
	BusExitNotFound = 3
	// BusExitNotInTeam is the exit code when an agent is not in a team.
	BusExitNotInTeam = 4
	// BusExitPermission is the exit code for permission errors.
	BusExitPermission = 5
	// BusExitConflict is the exit code for state conflicts.
	BusExitConflict = 6
	// BusExitInvalidState is the exit code for invalid state errors.
	BusExitInvalidState = 7
	// BusExitStorage is the exit code for storage or I/O errors.
	BusExitStorage = 8
	// BusExitUnknownTool is the exit code for unknown tools.
	BusExitUnknownTool = 9
	// BusExitProcess is the exit code for process errors.
	BusExitProcess = 10
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
	// BusDir is the path to the agentbus state directory.
	BusDir string
	// Agent is the name of the agent.
	Agent string
	// TaskID is the task identifier.
	TaskID string
	// TaskDir is the directory for the task.
	TaskDir string
	// RepoName is the repository name.
	RepoName string
	// Branch is the git branch name.
	Branch string
	// Worktree is the path to the git worktree.
	Worktree string
	// Peers is a comma-separated list of peer agent names.
	Peers string
	// Role is the agent role (worker, normal, or coordinator).
	Role string
	// QueueMax is the maximum size of the investigation queue.
	QueueMax int
	// InvSlots is the number of investigation slots available.
	InvSlots int
	// TasksDir is the directory containing tasks.
	TasksDir string
	// Team is the team identifier.
	Team string
	// Tier is the agent tier (worker or normal).
	Tier string
	// AgentCmd is the path to the agent command.
	AgentCmd string
	// AgentsHome is the agents home directory.
	AgentsHome string

	// Remote mode
	// RemoteTeam indicates whether this is a remote team.
	RemoteTeam bool
	// JobLead is the job lead relay URL for remote mode.
	JobLead string
	// Member is the member identifier for remote mode.
	Member string
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
func busMessagesFrom(path, agent string) (count int, err error) {
	file, openErr := os.Open(path) // #nosec G304 -- path is constructed by the agent runner
	if openErr != nil {
		if os.IsNotExist(openErr) {
			return 0, nil // File doesn't exist yet, count is 0
		}
		return 0, storageErr(openErr, "open messages.jsonl")
	}
	defer func() {
		if closeErr := file.Close(); closeErr != nil && err == nil {
			err = closeErr
		}
	}()

	scanner := bufio.NewScanner(file)
	const maxBufferSize = 1024 * 1024
	scanner.Buffer(make([]byte, bufio.MaxScanTokenSize), maxBufferSize)
	count = 0

	for scanner.Scan() {
		var msg map[string]interface{}
		if scanErr := json.Unmarshal(scanner.Bytes(), &msg); scanErr != nil {
			continue // Skip malformed lines
		}

		if from, ok := msg["from"].(string); ok && from == agent {
			count++
		}
	}

	if scanErr := scanner.Err(); scanErr != nil {
		return 0, storageErr(scanErr, "scan messages.jsonl")
	}

	return count, nil
}

// BusExecutor is a function type for executing agentbus commands.
// Used for testing and production execution of agentbus subcommands.
type BusExecutor func(cmd string, args []string) error

// sendNoticeWithExecutor sends a typed notice through the bus using the provided executor.
func sendNoticeWithExecutor(notice ScriptNotice, from, to, text string, fields map[string]string, executor BusExecutor) error {
	if !notice.Valid() {
		return fmt.Errorf("invalid notice: %s", notice)
	}

	args := []string{"send", "--from", from, "--to", to, "--notice", string(notice)}

	for key, value := range fields {
		args = append(args, "--field", fmt.Sprintf("%s=%s", key, value))
	}
	args = append(args, text)

	return executor("agentbus", args)
}
