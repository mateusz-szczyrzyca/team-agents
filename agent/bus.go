package main

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
	Cause BusCause
	Text  string
}

// Error implements the error interface.
func (e *BusError) Error() string {
	return e.Text
}

// Class returns the ErrorClass for this error.
func (e *BusError) Class() ErrorClass {
	// Stub - implementation required
	return 0
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
	// Stub - implementation required
	return nil
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
	// Stub - implementation required
	return nil
}

// busMessagesFrom counts messages from a specific agent in messages.jsonl.
func busMessagesFrom(path, agent string) (int, error) {
	// Stub - implementation required
	return 0, nil
}
