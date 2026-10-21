// Investigations: an agent (cheaper model) asks an advanced model to investigate a problem.
//
// Global queue: at most one investigation in progress per investigator (starting/running/waiting_human)
// + at most queueMax waiting (queued). When the queue is full, the agent either
// solves the problem itself (with a justification in record_decision) or goes to the
// waiting room (waitroom), from which the request enters the queue when a slot frees up.
// Starting investigators is done by the `agent` script (inv-next / inv-set).
//
// Every investigation has a markdown record file (question → status changes → result → decision). Writing
// it is a side effect: a failure is logged as a warning and does not fail the operation.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

// invSlots is the number of investigators (how many requests can be in progress at once). Set with --inv-slots.
var invSlots = 1

// invKey converts an investigation ID to its State map key.
func invKey(id int64) string { return strconv.FormatInt(id, 10) }

// sortedInv returns investigations ordered by ID (FIFO).
func sortedInv(st *State) []*Investigation {
	out := make([]*Investigation, 0, len(st.Investigations))
	for _, inv := range st.Investigations {
		out = append(out, inv)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// HasStarted reports whether an investigator has started working on the investigation.
func (inv *Investigation) HasStarted() bool { return !inv.Started.IsZero() }

// ---------------------------------------------------------------- queue arithmetic

// queueStats is the occupancy of the investigation queue.
type queueStats struct {
	Active   int // occupying an investigator (starting, running, waiting for the human)
	Queued   int // in the queue
	Waitroom int // in the waiting room (the queue was full)
}

// queueCounts counts investigations that are in progress, queued and in the waiting room.
func queueCounts(st *State) queueStats {
	var q queueStats
	for _, inv := range st.Investigations {
		switch {
		case inv.Status.Active():
			q.Active++
		case inv.Status == InvQueued:
			q.Queued++
		case inv.Status == InvWaitroom:
			q.Waitroom++
		}
	}
	return q
}

// freeSlots returns how many investigators are free (queued requests start immediately).
func freeSlots(st *State) int {
	if f := invSlots - queueCounts(st).Active; f > 0 {
		return f
	}
	return 0
}

// capacity returns how many requests may be "queued": the waiting limit + those that start right away.
func capacity(st *State, queueLimit int) int { return queueLimit + freeSlots(st) }

// waitingCount returns the requests waiting in the queue, excluding those that start right away.
func waitingCount(st *State) int {
	if w := queueCounts(st).Queued - freeSlots(st); w > 0 {
		return w
	}
	return 0
}

// promote moves requests from the waiting room to the queue (FIFO) while there is room. Announcing the
// move is a side effect of the operation that triggered the promotion, so failures are logged.
func (b *Bus) promote(st *State, queueLimit int) {
	queued := queueCounts(st).Queued
	limit := capacity(st, queueLimit)
	for _, inv := range sortedInv(st) {
		if queued >= limit {
			return
		}
		if inv.Status != InvWaitroom {
			continue
		}
		inv.Status = InvQueued
		queued++
		inv.record(recordPromoted(Now()))
		if _, serr := b.post(st, "agentbus", "all", EventInvestigationPromoted{ID: inv.ID, Requester: inv.FromAgent}); serr != nil {
			warnSideEffect("announce promotion", serr, "investigation", inv.ID)
		}
	}
}

// ---------------------------------------------------------------- status transitions

// transition changes the status of an investigation (as reported by the `agent` script) and returns the
// event that announces the change, or nil when the change needs no announcement.
func (inv *Investigation) transition(to InvestigationStatus, at Timestamp) busEvent {
	inv.Status = to
	switch to {
	case InvRunning:
		if !inv.HasStarted() {
			inv.Started = at
			return EventInvestigationStarted{ID: inv.ID, Requester: inv.FromAgent, Question: inv.Question}
		}
		return EventInvestigationResumed{ID: inv.ID}
	case InvWaitingHuman:
		return EventInvestigationWaiting{ID: inv.ID}
	case InvDone, InvFailed:
		inv.Finished = at
		return EventInvestigationFinished{ID: inv.ID, Status: to, Summary: inv.Summary}
	case InvQueued:
		return EventInvestigationRequeued{ID: inv.ID}
	}
	return nil
}

// ---------------------------------------------------------------- the investigation record file

// record appends an entry to the investigation's markdown record. The record is auxiliary: a write failure
// is logged and does not fail the operation that produced the entry.
func (inv *Investigation) record(entry string) {
	if serr := appendFile(inv.File, entry); serr != nil {
		warnSideEffect("write investigation record", serr, "investigation", inv.ID, "file", inv.File)
	}
}

// recordHeader is the first entry of an investigation record.
func recordHeader(inv *Investigation) string {
	return fmt.Sprintf("# Investigation #%d\n\n- Requested by: %s (task %s)\n- Repo: %s, agent branch: %s\n- Agent worktree: %s\n- Created: %s\n",
		inv.ID, inv.FromAgent, inv.FromTask, inv.Repo, inv.Branch, inv.Worktree, inv.Created)
}

// recordReviewHeader is the first entry of a PR review record.
func recordReviewHeader(inv *Investigation) string {
	return fmt.Sprintf("# PR review #%d (request #%d)\n\n- PR: %s\n- Team / task: %s (%s)\n- Branch: %s\n- Created: %s, status: %s\n",
		inv.PRNumber, inv.ID, inv.PRURL, inv.FromTask, inv.FromAgent, inv.Branch, inv.Created, inv.Status)
}

// recordQuestion records the question and the context given by the agent.
func recordQuestion(question, context string) string {
	return fmt.Sprintf("\n## Question from the agent\n\n%s\n\n## Context from the agent\n\n%s\n", question, orDash(context))
}

// recordRetried records that a rejected request was submitted again.
func recordRetried(at Timestamp, wait bool) string {
	return fmt.Sprintf("\n- %s: request retried (wait_if_full=%v)\n", at, wait)
}

// recordRequestStatus records how a request was handled by the queue.
func recordRequestStatus(at Timestamp, status InvestigationStatus, active, waiting, queueMax int) string {
	return fmt.Sprintf("\n- %s: request status: %s (in progress: %d, waiting: %d, queue limit: %d)\n", at, status, active, waiting, queueMax)
}

// recordPromoted records a move from the waiting room to the queue.
func recordPromoted(at Timestamp) string {
	return fmt.Sprintf("\n- %s: moved from the waiting room to the queue\n", at)
}

// recordStatus records a status change reported by the `agent` script.
func recordStatus(at Timestamp, status InvestigationStatus) string {
	return fmt.Sprintf("\n- %s: status → %s\n", at, status)
}

// recordDecision records the requester's decision.
func recordDecision(agent string, at Timestamp, decision, justification string) string {
	return fmt.Sprintf("\n## Agent decision (%s, %s)\n\n**Decision:** %s\n\n**Justification:** %s\n", agent, at, decision, justification)
}

// appendFile appends text to a file, creating parent directories. Every failure (directory, open, write,
// close) is a *StorageError; callers decide whether it fails their operation or is a logged side effect.
func appendFile(path, text string) (serr *StorageError) {
	if path == "" {
		return storageErr(errors.New("empty path"), "cannot append to a file")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return storageErr(err, "cannot create the directory of %s", path)
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600) //nolint:gosec // path is under AGENTS_HOME, which only the operator controls
	if err != nil {
		return storageErr(err, "cannot open %s", path)
	}
	defer func() {
		if err := f.Close(); err != nil && serr == nil {
			serr = storageErr(err, "cannot close %s", path)
		}
	}()
	if _, err := f.WriteString(text); err != nil {
		return storageErr(err, "cannot write to %s", path)
	}
	return nil
}

// trunc collapses whitespace and shortens s to n runes (with an ellipsis).
func trunc(s string, n int) string {
	s = strings.Join(strings.Fields(s), " ")
	r := []rune(s)
	if len(r) <= n {
		return s
	}
	return string(r[:n]) + "…"
}

// orDash returns "—" for blank strings.
func orDash(s string) string {
	if strings.TrimSpace(s) == "" {
		return "—"
	}
	return s
}

// fmtInv renders one investigation as a single line.
func fmtInv(inv *Investigation) string {
	line := fmt.Sprintf("#%d [%s] from %s (task %s): %s", inv.ID, inv.Status, inv.FromAgent, inv.FromTask, trunc(inv.Question, 120))
	if inv.InvTask != "" {
		line += " → investigator task " + inv.InvTask
	}
	return line
}

// ---------------------------------------------------------------- MCP tools

// Replies of request_investigation. They are normal tool results (not errors): the agent acts on them.
const (
	// replyAlreadyPending: the task already has a pending investigation.
	replyAlreadyPending = "You already have a pending investigation #%d (status: %s). Do not request another — " +
		"end your turn with STATUS: INVESTIGATING and wait for the result."
	// replyAccepted: the request is in the queue (it starts when an investigator is free).
	replyAccepted = "ACCEPTED: investigation #%d (%s). Now: commit your current work on your branch " +
		"(the investigator starts from its state), release reservations and END YOUR TURN with STATUS: INVESTIGATING. " +
		"The result arrives automatically in your next turn."
	// replyWaitroom: the queue is full and the agent chose to wait.
	replyWaitroom = "WAITING ROOM: investigation #%d is waiting for a queue slot (waiting: %d/%d). " +
		"It will enter the queue automatically. Commit your work, release reservations and END YOUR TURN with STATUS: INVESTIGATING."
	// replyQueueFull: the queue is full; the agent must decide (solve it itself or wait).
	replyQueueFull = "QUEUE FULL: investigation #%d was not accepted (investigators busy, waiting: %d/%d). Choose:\n" +
		"(a) solve the problem yourself — FIRST call record_decision(investigation_id=%d, decision, justification) " +
		"with a justification, then continue;\n" +
		"(b) wait — call request_investigation again with wait_if_full=true and end your turn."
)

// requestInvestigation (MCP request_investigation) queues a request for the investigators. When the queue
// is full the request is either parked in the waiting room (wait=true) or rejected, and the agent must
// record a decision. A rejected request without a decision is reused on retry.
func (s *Server) requestInvestigation(question, context string, wait bool) (string, *RequestInvestigationError) {
	if s.role == RoleInvestigator {
		return "", opErr(RequestInvestigationInvestigator, "an investigator cannot request investigations")
	}
	question = strings.TrimSpace(question)
	if question == "" {
		return "", opErr(RequestInvestigationMissingQuestion, "%s", missing("question"))
	}
	var msg string
	err := transact(s.bus, RequestInvestigationStorage, func(st *State) *RequestInvestigationError {
		s.touch(st)
		s.bus.promote(st, s.queueMax)

		var reuse *Investigation
		for _, inv := range sortedInv(st) {
			if inv.FromTask != s.info.Task {
				continue
			}
			if inv.Status.Pending() {
				msg = fmt.Sprintf(replyAlreadyPending, inv.ID, inv.Status)
				return nil
			}
			if inv.Status == InvRejected && inv.Decision == "" {
				reuse = inv // retry of a rejected request
			}
		}

		q := queueCounts(st)
		active, queued := q.Active, q.Queued
		limit := capacity(st, s.queueMax)
		waiting := waitingCount(st)
		inv := reuse
		if inv == nil {
			st.NextInvID++
			inv = &Investigation{
				ID: st.NextInvID, FromAgent: s.me, FromTask: s.info.Task, Repo: s.info.Repo,
				Branch: s.info.Branch, Worktree: s.info.Worktree, Created: Now(),
				File: filepath.Join(s.tasksDir, s.info.Task, "investigations", fmt.Sprintf("inv-%d.md", st.NextInvID)),
			}
			st.Investigations[invKey(inv.ID)] = inv
			inv.record(recordHeader(inv))
		} else {
			inv.record(recordRetried(Now(), wait))
		}
		inv.Question, inv.Context = question, strings.TrimSpace(context)
		inv.record(recordQuestion(inv.Question, inv.Context))

		switch {
		case queued < limit:
			inv.Status = InvQueued
			when := fmt.Sprintf("position %d in the waiting queue", waiting+1)
			if active < invSlots && queued < freeSlots(st) {
				when = "an investigator is free — it starts right after your turn ends"
			}
			msg = fmt.Sprintf(replyAccepted, inv.ID, when)
		case wait:
			inv.Status = InvWaitroom
			msg = fmt.Sprintf(replyWaitroom, inv.ID, waiting, s.queueMax)
		default:
			inv.Status = InvRejected
			msg = fmt.Sprintf(replyQueueFull, inv.ID, waiting, s.queueMax, inv.ID)
		}
		inv.record(recordRequestStatus(Now(), inv.Status, active, waiting, s.queueMax))
		_, serr := s.bus.post(st, s.me, "all", EventInvestigationRequested{ID: inv.ID, Status: inv.Status, Question: question})
		return fromStorage(RequestInvestigationStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return msg, nil
}

// recordDecision (MCP record_decision) stores the requester's decision after an investigation result or rejection.
func (s *Server) recordDecision(id int, decision, justification string) (string, *RecordDecisionError) {
	if s.role == RoleInvestigator {
		return "", opErr(RecordDecisionInvestigator, "record_decision is for the agent that requested the investigation")
	}
	decision, justification = strings.TrimSpace(decision), strings.TrimSpace(justification)
	if decision == "" || justification == "" {
		return "", opErr(RecordDecisionMissingArgument, "%s", missing("decision", "justification"))
	}
	err := transact(s.bus, RecordDecisionStorage, func(st *State) *RecordDecisionError {
		s.touch(st)
		inv := st.Investigations[invKey(int64(id))]
		switch {
		case inv == nil:
			return opErr(RecordDecisionNoSuchInvestigation, "no investigation #%d", id)
		case inv.FromTask != s.info.Task:
			return opErr(RecordDecisionNotYourInvestigation, "investigation #%d does not belong to your task", id)
		case !inv.Status.Finished():
			return opErr(RecordDecisionNotFinished, "investigation #%d is %s — record the decision after the result or after a rejection", id, inv.Status)
		}
		inv.Decision, inv.Justification, inv.DecidedAt = decision, justification, Now()
		inv.record(recordDecision(s.me, inv.DecidedAt, decision, justification))
		_, serr := s.bus.post(st, s.me, "all", EventDecisionRecorded{ID: inv.ID, Decision: decision})
		return fromStorage(RecordDecisionStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("Decision for #%d saved.", id), nil
}

// investigationStatus (MCP investigation_status) shows the queue and the caller's investigations.
func (s *Server) investigationStatus() (string, *InvestigationStatusError) {
	var sb strings.Builder
	err := transact(s.bus, InvestigationStatusStorage, func(st *State) *InvestigationStatusError {
		s.touch(st)
		q := queueCounts(st)
		fmt.Fprintf(&sb, "Investigation queue: in progress %d/%d, waiting %d/%d, waiting room %d\n", q.Active, invSlots, waitingCount(st), s.queueMax, q.Waitroom)
		for _, inv := range sortedInv(st) {
			if inv.Status.Pending() || inv.FromTask == s.info.Task {
				sb.WriteString("- " + fmtInv(inv) + "\n")
			}
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return sb.String(), nil
}

// invToolDefs returns the investigation MCP tools (not offered to investigators themselves).
func invToolDefs(role Role) []map[string]any {
	if role == RoleInvestigator {
		return nil
	}
	str := func(d string) map[string]any { return map[string]any{"type": "string", "description": d} }
	return []map[string]any{
		{
			"name": "request_investigation",
			"description": "Ask an advanced (expensive) investigator model to look into a hard problem. " +
				"The investigator has a shell and hardware access and starts from your branch. Once accepted (or in the waiting room) end your turn — " +
				"the result arrives automatically. If the queue is full: solve it yourself (record_decision with a justification) or retry with wait_if_full=true.",
			"inputSchema": map[string]any{"type": "object", "required": []string{"question"}, "properties": map[string]any{
				"question":     str("The specific question / problem to investigate"),
				"context":      str("What you already know and checked, hypotheses, relevant files, commands, logs"),
				"wait_if_full": map[string]any{"type": "boolean", "description": "true = wait in the waiting room when the queue is full"},
			}},
		},
		{
			"name":        "record_decision",
			"description": "Record your final decision after the investigation result (or after a rejection by a full queue) with a justification.",
			"inputSchema": map[string]any{"type": "object", "required": []string{"investigation_id", "decision", "justification"}, "properties": map[string]any{
				"investigation_id": map[string]any{"type": "integer", "description": "Investigation number"},
				"decision":         str("What you decide to do"),
				"justification":    str("Why — including whether and how you verified the investigator's findings"),
			}},
		},
		{
			"name":        "investigation_status",
			"description": "Show the investigation queue and your investigations.",
			"inputSchema": map[string]any{"type": "object", "properties": map[string]any{}},
		},
	}
}

// ---------------------------------------------------------------- CLI used by the `agent` script

// printJSON writes v as one JSON line to stdout. The `agent` script parses this output, so a write failure
// is a *StorageError that fails the subcommand.
func printJSON(v any) *StorageError {
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return storageErr(err, "cannot write JSON to stdout")
	}
	return nil
}

// inv-next: promotes the waiting room; if an investigator slot is free, marks the first queued request as
// "starting" and prints it as JSON (empty output = nothing to start).
func cmdInvNext(args []string) {
	fs := newFlagSet("inv-next")
	dir := fs.String("dir", defaultDir(), "state directory")
	queueMax := fs.Int("queue-max", 3, "queue limit")
	slots := fs.Int("inv-slots", 1, "number of investigators")
	parseFlags(fs, args)
	invSlots = *slots
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdInvNextStorage, func(st *State) *CmdInvNextError {
		b.promote(st, *queueMax)
		if queueCounts(st).Active >= invSlots {
			return nil
		}
		for _, inv := range sortedInv(st) {
			if inv.Status == InvQueued {
				inv.Status = InvStarting
				return fromStorage(CmdInvNextStorage, printJSON(inv))
			}
		}
		return nil
	}).asError())
}

// inv-set: updates an investigation (status, investigator task, summary) and announces a status change on the bus.
func cmdInvSet(args []string) {
	fs := newFlagSet("inv-set")
	dir := fs.String("dir", defaultDir(), "state directory")
	id := fs.Int64("id", 0, "investigation number")
	status := fs.String("status", "", "new status")
	invTask := fs.String("inv-task", "", "investigator task ID")
	summary := fs.String("summary", "", "short result summary")
	from := fs.String("from", "investigator", "sender of the bus message")
	parseFlags(fs, args)
	var newStatus InvestigationStatus
	if *status != "" {
		v, verr := parseEnum[InvestigationStatus](*status, "investigation status")
		exitOn(fromValue(CmdInvSetInvalidStatus, verr).asError())
		newStatus = v
	}
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdInvSetStorage, func(st *State) *CmdInvSetError {
		inv := st.Investigations[invKey(*id)]
		if inv == nil {
			return opErr(CmdInvSetNoSuchInvestigation, "no investigation #%d", *id)
		}
		if *invTask != "" {
			inv.InvTask = *invTask
		}
		if *summary != "" {
			inv.Summary = *summary
		}
		if newStatus == "" || newStatus == inv.Status {
			return nil
		}
		at := Now()
		announcement := inv.transition(newStatus, at)
		inv.record(recordStatus(at, newStatus))
		if announcement == nil {
			return nil
		}
		if _, serr := b.post(st, *from, "all", announcement); serr != nil {
			e := fromStorage(CmdInvSetStorage, serr)
			e.Message = fmt.Sprintf("investigation #%d is now %s but announcing it failed: %s", inv.ID, newStatus, serr.Message)
			return e
		}
		return nil
	}).asError())
}

// inv-get: prints an investigation as JSON.
func cmdInvGet(args []string) {
	fs := newFlagSet("inv-get")
	dir := fs.String("dir", defaultDir(), "state directory")
	id := fs.Int64("id", 0, "investigation number")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdInvGetStorage, func(st *State) *CmdInvGetError {
		inv := st.Investigations[invKey(*id)]
		if inv == nil {
			return opErr(CmdInvGetNoSuchInvestigation, "no investigation #%d", *id)
		}
		return fromStorage(CmdInvGetStorage, printJSON(inv))
	}).asError())
}

// inv-pending: prints the number of the task's pending investigation (if any).
func cmdInvPending(args []string) {
	fs := newFlagSet("inv-pending")
	dir := fs.String("dir", defaultDir(), "state directory")
	task := fs.String("task", "", "agent task ID")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdInvPendingStorage, func(st *State) *CmdInvPendingError {
		for _, inv := range sortedInv(st) {
			if inv.FromTask == *task && inv.Status.Pending() {
				out.println(strconv.FormatInt(inv.ID, 10))
				break
			}
		}
		return fromStorage(CmdInvPendingStorage, out.err)
	}).asError())
}

// inv-list: human-readable queue and investigation list.
func cmdInvList(args []string) {
	fs := newFlagSet("inv-list")
	dir := fs.String("dir", defaultDir(), "state directory")
	all := fs.Bool("all", false, "include finished")
	queueMax := fs.Int("queue-max", 3, "queue limit")
	task := fs.String("task", "", "only investigations of this task (no header)")
	slots := fs.Int("inv-slots", 1, "number of investigators")
	parseFlags(fs, args)
	invSlots = *slots
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdInvListStorage, func(st *State) *CmdInvListError {
		if *task == "" {
			q := queueCounts(st)
			out.printf("Queue: in progress %d/%d, waiting %d/%d, waiting room %d\n", q.Active, invSlots, waitingCount(st), *queueMax, q.Waitroom)
		}
		for _, inv := range sortedInv(st) {
			if *task != "" && inv.FromTask != *task {
				continue
			}
			if *all || *task != "" || inv.Status.Pending() {
				line := fmtInv(inv)
				if inv.Decision != "" {
					line += " | decision: " + trunc(inv.Decision, 80)
				}
				out.println("  " + line)
			}
		}
		return fromStorage(CmdInvListStorage, out.err)
	}).asError())
}

// inv-add: a request for an investigator from the script (e.g. a PR review after --verify). It goes to the end
// of the same queue as investigations; when the queue is full — to the waiting room (it enters automatically).
func cmdInvAdd(args []string) {
	fs := newFlagSet("inv-add")
	dir := fs.String("dir", defaultDir(), "state directory")
	queueMax := fs.Int("queue-max", 3, "queue limit")
	kind := fs.String("kind", string(KindReview), "request kind")
	fromAgent := fs.String("from-agent", "", "requesting agent")
	fromTask := fs.String("from-task", "", "requesting task")
	repo := fs.String("repo", "", "repository")
	branch := fs.String("branch", "", "branch")
	tasksDir := fs.String("tasks-dir", "", "tasks directory")
	question := fs.String("question", "", "request text")
	prURL := fs.String("pr-url", "", "pull request URL")
	prNum := fs.Int("pr-number", 0, "PR number")
	slots := fs.Int("inv-slots", 1, "number of investigators")
	parseFlags(fs, args)
	invSlots = *slots
	k, verr := parseEnum[InvestigationKind](*kind, "investigation kind")
	exitOn(fromValue(CmdInvAddInvalidKind, verr).asError())
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdInvAddStorage, func(st *State) *CmdInvAddError {
		b.promote(st, *queueMax)
		queued := queueCounts(st).Queued
		st.NextInvID++
		inv := &Investigation{
			ID: st.NextInvID, FromAgent: *fromAgent, FromTask: *fromTask, Repo: *repo, Branch: *branch,
			Question: *question, Created: Now(), Kind: k, PRURL: *prURL, PRNumber: *prNum, Status: InvQueued,
			File: filepath.Join(*tasksDir, *fromTask, "investigations", fmt.Sprintf("inv-%d.md", st.NextInvID)),
		}
		if queued >= capacity(st, *queueMax) {
			inv.Status = InvWaitroom
		}
		st.Investigations[invKey(inv.ID)] = inv
		inv.record(recordReviewHeader(inv))
		if _, serr := b.post(st, "agentbus", "all", EventReviewQueued{ID: inv.ID, Status: inv.Status, PRURL: inv.PRURL}); serr != nil {
			return fromStorage(CmdInvAddStorage, serr)
		}
		out.println(strconv.FormatInt(inv.ID, 10))
		return fromStorage(CmdInvAddStorage, out.err)
	}).asError())
}

// cancel-pending: cancels all unfinished investigations/reviews and worker requests and closes all teams
// (agent stop --running).
func cmdCancelPending(args []string) {
	fs := newFlagSet("cancel-pending")
	dir := fs.String("dir", defaultDir(), "state directory")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdCancelPendingStorage, func(st *State) *CmdCancelPendingError {
		var ev EventStopAll
		for _, inv := range st.Investigations {
			if inv.Status.Pending() {
				inv.Status, inv.Summary, inv.Finished = InvFailed, "cancelled (agent stop --running)", Now()
				ev.Investigations++
			}
		}
		for _, h := range st.Helpers {
			if h.Status.Open() {
				h.Status, h.Summary = HelperFailed, "cancelled (agent stop --running)"
				ev.Requests++
			}
		}
		for _, t := range st.Teams {
			if !t.Done {
				t.Done = true
				ev.Teams++
			}
		}
		if _, serr := b.post(st, "agentbus", "all", ev); serr != nil {
			return fromStorage(CmdCancelPendingStorage, serr)
		}
		out.printf("cancelled: investigations/reviews %d, worker requests %d, teams %d\n", ev.Investigations, ev.Requests, ev.Teams)
		return fromStorage(CmdCancelPendingStorage, out.err)
	}).asError())
}
