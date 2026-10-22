// Bus events: every message agentbus publishes is a typed event.
//
// An event type carries the facts of what happened (IDs, statuses, names) and renders the text that agents and
// humans read. Bus.post stores both in messages.jsonl:
//
//	{"id":7,"ts":"…","from":"agentbus","to":"all","kind":"investigation_finished",
//	 "data":{"id":3,"status":"done","summary":"…"},"text":"INVESTIGATION #3 finished [done]: …"}
//
// so log pipelines (Loki, VictoriaLogs) can filter by kind and fields, while agents keep reading plain text.
// Messages written by agents or humans are EventChat; events of the `agent` script are EventScript with a
// ScriptNotice kind. Messages from older versions have no kind and are treated as chat.
package main

import (
	"fmt"
	"strings"
)

// MessageKind identifies the type of a bus message.
type MessageKind string

const (
	KindChat                   MessageKind = "chat"                    // free text from an agent or a human
	KindReserved               MessageKind = "reserved"                // a resource was reserved
	KindReleased               MessageKind = "released"                // a resource was released by its holder
	KindAutoReleased           MessageKind = "auto_released"           // a reservation of a gone/finished agent was released
	KindForcedRelease          MessageKind = "forced_release"          // a human released someone else's reservation
	KindInvestigationRequested MessageKind = "investigation_requested" // an agent requested an investigation
	KindInvestigationPromoted  MessageKind = "investigation_promoted"  // moved from the waiting room to the queue
	KindInvestigationStarted   MessageKind = "investigation_started"   // an investigator started working on it
	KindInvestigationResumed   MessageKind = "investigation_resumed"   // an investigator resumed after the human's answers
	KindInvestigationWaiting   MessageKind = "investigation_waiting"   // the investigator waits for the human's answers
	KindInvestigationRequeued  MessageKind = "investigation_requeued"  // put back into the queue
	KindInvestigationFinished  MessageKind = "investigation_finished"  // finished (done or failed)
	KindDecisionRecorded       MessageKind = "decision_recorded"       // the requester recorded its decision
	KindReviewQueued           MessageKind = "review_queued"           // a PR review was queued (--verify)
	KindTeamCreated            MessageKind = "team_created"            // a team was created
	KindTeamResumed            MessageKind = "team_resumed"            // a team was resumed (agent run --resume)
	KindTeamFinished           MessageKind = "team_finished"           // the team task is finished
	KindCoordinatorClaimed     MessageKind = "coordinator_claimed"     // a member became the coordinator
	KindCoordinatorTakeover    MessageKind = "coordinator_takeover"    // a member replaced a hung coordinator
	KindCoordinatorHandover    MessageKind = "coordinator_handover"    // the coordinator handed the role over
	KindQuizQuestion           MessageKind = "quiz_question"           // the coordinator asked the qualification question
	KindExcludedNotice         MessageKind = "excluded_notice"         // tells the excluded member to stop
	KindMemberExcluded         MessageKind = "member_excluded"         // tells the team that a member was excluded
	KindBoardItemAdded         MessageKind = "board_item_added"        // a board item was added
	KindBoardResult            MessageKind = "board_result"            // a board item was completed (sent to its creator)
	KindBoardReturned          MessageKind = "board_returned"          // a board item was returned (sent to its creator)
	KindWorkerRequest          MessageKind = "worker_request"          // a delegate request was queued
	KindCheckpoint             MessageKind = "checkpoint"              // a checkpoint was saved
	KindStopAll                MessageKind = "stop_all"                // the human stopped all work
	KindScript                 MessageKind = "script"                  // an event of the `agent` script (see ScriptNotice)
)

// Valid reports whether k is a defined kind ("" is accepted for messages written by older versions).
func (k MessageKind) Valid() bool {
	switch k {
	case "", KindChat, KindReserved, KindReleased, KindAutoReleased, KindForcedRelease,
		KindInvestigationRequested, KindInvestigationPromoted, KindInvestigationStarted, KindInvestigationResumed,
		KindInvestigationWaiting, KindInvestigationRequeued, KindInvestigationFinished, KindDecisionRecorded,
		KindReviewQueued, KindTeamCreated, KindTeamResumed, KindTeamFinished, KindCoordinatorClaimed,
		KindCoordinatorTakeover, KindCoordinatorHandover, KindQuizQuestion, KindExcludedNotice, KindMemberExcluded,
		KindBoardItemAdded, KindBoardResult, KindBoardReturned, KindWorkerRequest, KindCheckpoint, KindStopAll,
		KindScript:
		return true
	}
	return false
}

// busEvent is implemented by every event type. Kind identifies it; Text renders the message people read.
type busEvent interface {
	Kind() MessageKind
	Text() string
}

// ---------------------------------------------------------------- chat

// EventChat is free text written by an agent (send_message) or a human (agent say / agentbus send).
type EventChat struct {
	Body string `json:"body"` // the message as written
}

func (e EventChat) Kind() MessageKind { return KindChat } // Kind implements busEvent.
func (e EventChat) Text() string      { return e.Body }   // Text implements busEvent.

// ---------------------------------------------------------------- reservations

// EventReserved announces a new reservation.
type EventReserved struct {
	Resource string `json:"resource"`       // reserved resource
	Note     string `json:"note,omitempty"` // why it was reserved
}

func (e EventReserved) Kind() MessageKind { return KindReserved } // Kind implements busEvent.

// Text implements busEvent.
func (e EventReserved) Text() string {
	if e.Note == "" {
		return "RESERVING " + e.Resource
	}
	return "RESERVING " + e.Resource + ": " + e.Note
}

// EventReleased announces that the holder released a resource.
type EventReleased struct {
	Resource string `json:"resource"` // released resource
}

func (e EventReleased) Kind() MessageKind { return KindReleased }              // Kind implements busEvent.
func (e EventReleased) Text() string      { return "RELEASING " + e.Resource } // Text implements busEvent.

// AutoReleaseReason says why a reservation was released automatically.
type AutoReleaseReason string

const (
	ReleaseAgentGone     AutoReleaseReason = "agent_gone"     // the holder's agentbus process is no longer running
	ReleaseAgentFinished AutoReleaseReason = "agent_finished" // the holder's run ended normally
)

// EventAutoReleased announces a reservation released because its holder is gone or finished.
type EventAutoReleased struct {
	Resource string            `json:"resource"` // released resource
	Holder   string            `json:"holder"`   // the agent that held it
	Reason   AutoReleaseReason `json:"reason"`   // why it was released
}

func (e EventAutoReleased) Kind() MessageKind { return KindAutoReleased } // Kind implements busEvent.

// Text implements busEvent.
func (e EventAutoReleased) Text() string {
	why := "is no longer running"
	if e.Reason == ReleaseAgentFinished {
		why = "finished"
	}
	return fmt.Sprintf("AUTO-RELEASE %s (agent %s %s)", e.Resource, e.Holder, why)
}

// EventForcedRelease announces that a human released someone else's reservation (agentbus release).
type EventForcedRelease struct {
	Resource string `json:"resource"` // released resource
	Holder   string `json:"holder"`   // the agent that held it
}

func (e EventForcedRelease) Kind() MessageKind { return KindForcedRelease } // Kind implements busEvent.

// Text implements busEvent.
func (e EventForcedRelease) Text() string {
	return fmt.Sprintf("FORCED RELEASE %s (held by %s)", e.Resource, e.Holder)
}

// ---------------------------------------------------------------- investigations

// EventInvestigationRequested announces a new request and whether it was accepted.
type EventInvestigationRequested struct {
	ID       int64               `json:"id"`       // investigation number
	Status   InvestigationStatus `json:"status"`   // queued, waitroom or rejected
	Question string              `json:"question"` // the question (shortened)
}

func (e EventInvestigationRequested) Kind() MessageKind { return KindInvestigationRequested } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationRequested) Text() string {
	return fmt.Sprintf("INVESTIGATION #%d [%s]: %s", e.ID, e.Status, trunc(e.Question, 200))
}

// EventInvestigationPromoted announces a request moved from the waiting room to the queue.
type EventInvestigationPromoted struct {
	ID        int64  `json:"id"`        // investigation number
	Requester string `json:"requester"` // the requesting agent
}

func (e EventInvestigationPromoted) Kind() MessageKind { return KindInvestigationPromoted } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationPromoted) Text() string {
	return fmt.Sprintf("INVESTIGATION #%d (from %s) moved from the waiting room to the queue", e.ID, e.Requester)
}

// EventInvestigationStarted announces that an investigator started working.
type EventInvestigationStarted struct {
	ID        int64  `json:"id"`        // investigation number
	Requester string `json:"requester"` // the requesting agent
	Question  string `json:"question"`  // the question (shortened)
}

func (e EventInvestigationStarted) Kind() MessageKind { return KindInvestigationStarted } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationStarted) Text() string {
	return fmt.Sprintf("INVESTIGATOR started #%d (from %s): %s", e.ID, e.Requester, trunc(e.Question, 160))
}

// EventInvestigationResumed announces that an investigator resumed after the human's answers.
type EventInvestigationResumed struct {
	ID int64 `json:"id"` // investigation number
}

func (e EventInvestigationResumed) Kind() MessageKind { return KindInvestigationResumed } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationResumed) Text() string {
	return fmt.Sprintf("INVESTIGATOR resumed #%d", e.ID)
}

// EventInvestigationWaiting announces that an investigator asked the human and waits (its slot is blocked).
type EventInvestigationWaiting struct {
	ID int64 `json:"id"` // investigation number
}

func (e EventInvestigationWaiting) Kind() MessageKind { return KindInvestigationWaiting } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationWaiting) Text() string {
	return fmt.Sprintf("INVESTIGATOR #%d is waiting for human answers (its slot is blocked)", e.ID)
}

// EventInvestigationRequeued announces that an investigation was put back into the queue.
type EventInvestigationRequeued struct {
	ID int64 `json:"id"` // investigation number
}

func (e EventInvestigationRequeued) Kind() MessageKind { return KindInvestigationRequeued } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationRequeued) Text() string {
	return fmt.Sprintf("INVESTIGATION #%d returns to the queue", e.ID)
}

// EventInvestigationFinished announces the end of an investigation.
type EventInvestigationFinished struct {
	ID      int64               `json:"id"`      // investigation number
	Status  InvestigationStatus `json:"status"`  // done or failed
	Summary string              `json:"summary"` // one-line result summary
}

func (e EventInvestigationFinished) Kind() MessageKind { return KindInvestigationFinished } // Kind implements busEvent.

// Text implements busEvent.
func (e EventInvestigationFinished) Text() string {
	return fmt.Sprintf("INVESTIGATION #%d finished [%s]: %s", e.ID, e.Status, trunc(e.Summary, 200))
}

// EventDecisionRecorded announces the requester's decision after an investigation.
type EventDecisionRecorded struct {
	ID       int64  `json:"id"`       // investigation number
	Decision string `json:"decision"` // the decision (shortened)
}

func (e EventDecisionRecorded) Kind() MessageKind { return KindDecisionRecorded } // Kind implements busEvent.

// Text implements busEvent.
func (e EventDecisionRecorded) Text() string {
	return fmt.Sprintf("DECISION after investigation #%d: %s", e.ID, trunc(e.Decision, 200))
}

// EventReviewQueued announces a PR review queued for an investigator (--verify).
type EventReviewQueued struct {
	ID     int64               `json:"id"`     // investigation number of the review
	Status InvestigationStatus `json:"status"` // queued or waitroom
	PRURL  string              `json:"pr_url"` // the pull request
}

func (e EventReviewQueued) Kind() MessageKind { return KindReviewQueued } // Kind implements busEvent.

// Text implements busEvent.
func (e EventReviewQueued) Text() string {
	return fmt.Sprintf("REVIEW #%d [%s]: %s", e.ID, e.Status, e.PRURL)
}

// ---------------------------------------------------------------- teams

// EventTeamCreated announces a new team.
type EventTeamCreated struct {
	Team    string   `json:"team"`    // team ID
	Members []string `json:"members"` // member names
}

func (e EventTeamCreated) Kind() MessageKind { return KindTeamCreated } // Kind implements busEvent.

// Text implements busEvent.
func (e EventTeamCreated) Text() string {
	return fmt.Sprintf("TEAM %s: %s — same task, agree on a coordinator (claim_lead)", e.Team, strings.Join(e.Members, ", "))
}

// EventTeamResumed announces a team resumed from an interrupted one (agent run --resume).
type EventTeamResumed struct {
	Team     string   `json:"team"`           // the new team ID
	Previous string   `json:"previous"`       // the interrupted team ID
	Members  []string `json:"members"`        // member names
	Lead     string   `json:"lead,omitempty"` // the preserved coordinator ("" = to be elected)
}

func (e EventTeamResumed) Kind() MessageKind { return KindTeamResumed } // Kind implements busEvent.

// Text implements busEvent.
func (e EventTeamResumed) Text() string {
	lead := e.Lead
	if lead == "" {
		lead = "to be decided (claim_lead)"
	}
	return fmt.Sprintf("TEAM %s RESUMED from %s: %s — coordinator: %s, no quiz, continue from the last checkpoint",
		e.Team, e.Previous, strings.Join(e.Members, ", "), lead)
}

// EventTeamFinished announces that the team task is finished.
type EventTeamFinished struct {
	Team string `json:"team"` // team ID
}

func (e EventTeamFinished) Kind() MessageKind { return KindTeamFinished }               // Kind implements busEvent.
func (e EventTeamFinished) Text() string      { return "TEAM " + e.Team + " finished" } // Text implements busEvent.

// EventCoordinatorClaimed announces the elected coordinator.
type EventCoordinatorClaimed struct {
	Coordinator string `json:"coordinator"` // the new coordinator
}

func (e EventCoordinatorClaimed) Kind() MessageKind { return KindCoordinatorClaimed } // Kind implements busEvent.

// Text implements busEvent.
func (e EventCoordinatorClaimed) Text() string {
	return "COORDINATOR: " + e.Coordinator + " takes over team coordination"
}

// EventCoordinatorTakeover announces that a member replaced a hung coordinator.
type EventCoordinatorTakeover struct {
	Coordinator string `json:"coordinator"` // the new coordinator
	Previous    string `json:"previous"`    // the hung coordinator
}

func (e EventCoordinatorTakeover) Kind() MessageKind { return KindCoordinatorTakeover } // Kind implements busEvent.

// Text implements busEvent.
func (e EventCoordinatorTakeover) Text() string {
	return "COORDINATOR: " + e.Coordinator + " takes over coordination from hung " + e.Previous
}

// EventCoordinatorHandover announces that the coordinator handed the role over.
type EventCoordinatorHandover struct {
	From string `json:"from"` // the previous coordinator
	To   string `json:"to"`   // the new coordinator
}

func (e EventCoordinatorHandover) Kind() MessageKind { return KindCoordinatorHandover } // Kind implements busEvent.

// Text implements busEvent.
func (e EventCoordinatorHandover) Text() string {
	return fmt.Sprintf("COORDINATOR: %s hands coordination over → %s", e.From, e.To)
}

// EventQuizQuestion is the qualification question sent to the team.
type EventQuizQuestion struct {
	Question string `json:"question"` // the question, including the required answer format
	Seconds  int    `json:"seconds"`  // time limit for answers
}

func (e EventQuizQuestion) Kind() MessageKind { return KindQuizQuestion } // Kind implements busEvent.

// Text implements busEvent.
func (e EventQuizQuestion) Text() string {
	return fmt.Sprintf("QUIZ (qualification for this task): %s\nAnswer with the quiz_answer tool within %d s — the answer only, "+
		"no commentary. You may use tools.", e.Question, e.Seconds)
}

// EventExcludedNotice tells an excluded member to stop working on the task.
type EventExcludedNotice struct {
	Team   string `json:"team"`   // team ID
	Reason string `json:"reason"` // why it was excluded
}

func (e EventExcludedNotice) Kind() MessageKind { return KindExcludedNotice } // Kind implements busEvent.

// Text implements busEvent.
func (e EventExcludedNotice) Text() string {
	return "EXCLUDED from team " + e.Team + ": " + e.Reason + ". Do no further work on this task — end your turn with STATUS: EXCLUDED."
}

// EventMemberExcluded tells the team that a member was excluded.
type EventMemberExcluded struct {
	Member string `json:"member"` // the excluded member
	Reason string `json:"reason"` // why it was excluded
}

func (e EventMemberExcluded) Kind() MessageKind { return KindMemberExcluded } // Kind implements busEvent.

// Text implements busEvent.
func (e EventMemberExcluded) Text() string {
	return fmt.Sprintf("TEAM: %s excluded by the coordinator (%s)", e.Member, e.Reason)
}

// EventBoardItemAdded announces a new board item.
type EventBoardItemAdded struct {
	ID    string     `json:"id"`            // item ID ("B<n>")
	Level BoardLevel `json:"level"`         // who should take it
	Title string     `json:"title"`         // short description
	For   string     `json:"for,omitempty"` // the only member that may claim it ("" = anyone at its level)
}

func (e EventBoardItemAdded) Kind() MessageKind { return KindBoardItemAdded } // Kind implements busEvent.

// Text implements busEvent.
func (e EventBoardItemAdded) Text() string {
	if e.For != "" {
		return fmt.Sprintf("BOARD %s [%s, for %s]: %s", e.ID, e.Level, e.For, e.Title)
	}
	return fmt.Sprintf("BOARD %s [%s]: %s", e.ID, e.Level, e.Title)
}

// EventBoardResult sends the result of a board item to its creator.
type EventBoardResult struct {
	ID     string `json:"id"`     // item ID
	Title  string `json:"title"`  // item title
	Result string `json:"result"` // the submitted result
}

func (e EventBoardResult) Kind() MessageKind { return KindBoardResult } // Kind implements busEvent.

// Text implements busEvent.
func (e EventBoardResult) Text() string {
	return fmt.Sprintf("RESULT %s (%s): %s", e.ID, e.Title, e.Result)
}

// EventBoardReturned tells the creator that an item was returned to the board.
type EventBoardReturned struct {
	ID    string `json:"id"`    // item ID
	Title string `json:"title"` // item title
	Note  string `json:"note"`  // why it was returned
}

func (e EventBoardReturned) Kind() MessageKind { return KindBoardReturned } // Kind implements busEvent.

// Text implements busEvent.
func (e EventBoardReturned) Text() string {
	return fmt.Sprintf("RETURNING %s (%s): %s", e.ID, e.Title, e.Note)
}

// ---------------------------------------------------------------- worker requests, checkpoints, stop

// EventWorkerRequest announces a queued delegate request.
type EventWorkerRequest struct {
	ID      int64  `json:"id"`      // request number (H<ID>)
	Request string `json:"request"` // the request (shortened)
}

func (e EventWorkerRequest) Kind() MessageKind { return KindWorkerRequest } // Kind implements busEvent.

// Text implements busEvent.
func (e EventWorkerRequest) Text() string {
	return fmt.Sprintf("REQUEST H%d for a worker: %s", e.ID, trunc(e.Request, 160))
}

// EventCheckpoint announces a saved checkpoint.
type EventCheckpoint struct {
	Summary string `json:"summary"` // the checkpoint summary (shortened)
}

func (e EventCheckpoint) Kind() MessageKind { return KindCheckpoint }                         // Kind implements busEvent.
func (e EventCheckpoint) Text() string      { return "CHECKPOINT: " + trunc(e.Summary, 160) } // Text implements busEvent.

// EventStopAll announces that the human stopped all work (agent stop --running).
type EventStopAll struct {
	Investigations int `json:"investigations"` // cancelled investigations/reviews
	Requests       int `json:"requests"`       // cancelled worker requests
	Teams          int `json:"teams"`          // closed teams
}

func (e EventStopAll) Kind() MessageKind { return KindStopAll } // Kind implements busEvent.

// Text implements busEvent.
func (e EventStopAll) Text() string { return "STOP: the human stopped all work (agent stop --running)" }

// ---------------------------------------------------------------- events of the agent script

// ScriptNotice is the kind of an event published by the `agent` script (agentbus send --notice).
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
	NoticeMemberPaused       ScriptNotice = "member_paused"       // a member is paused (quota or wake-loop guard) until a given time
)

// Valid reports whether n is a defined notice.
func (n ScriptNotice) Valid() bool {
	switch n {
	case NoticeMemberFinished, NoticeMemberFailed, NoticeMemberStopped, NoticeMemberQuestions, NoticeMemberNoStatus,
		NoticeTeamStuck, NoticeWorkerHung, NoticeAgentHung, NoticeCoordinatorHung, NoticeHelperResult, NoticeHelperStartFailure,
		NoticeMemberPaused:
		return true
	}
	return false
}

// EventScript is an event of the `agent` script. The script renders the text itself.
type EventScript struct {
	Notice ScriptNotice      `json:"notice"`           // what happened
	Fields map[string]string `json:"fields,omitempty"` // structured details (task, agent, team, ...)
	Body   string            `json:"body"`             // the text for agents and humans
}

func (e EventScript) Kind() MessageKind { return KindScript } // Kind implements busEvent.
func (e EventScript) Text() string      { return e.Body }     // Text implements busEvent.
