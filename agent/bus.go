package main

import (
	"context"
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
	return false
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
	return nil
}

// busArgsJSON constructs the argv list for agentbus serve matching bash bus_args_json.
func busArgsJSON(params BusServeParams) []string {
	return nil
}

// busMessagesFrom counts messages matching from: agent in messages.jsonl.
func busMessagesFrom(messagesPath string, agent string) (int, error) {
	return 0, nil
}

// busCmd executes an agentbus subcommand via the app executor and maps its status code.
func busCmd(ctx context.Context, app *App, busBin string, args ...string) (string, *BusError) {
	return "", nil
}
