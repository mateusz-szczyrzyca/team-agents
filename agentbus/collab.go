// Collaboration: qualification quiz, team task board and worker requests.
//
// Quiz: the coordinator asks one precise question (quiz_start), members answer
// privately (quiz_answer — answers do NOT go to the bus), the coordinator grades
// (quiz_results) and excludes those who failed (team_exclude).
//
// Board: team subtasks (easy — for workers, normal — for normal models).
// Adding an item wakes one sleeping member who can take it.
//
// Requests (delegate): a normal agent — also outside a team — hands a simple task to a free
// worker; the `agent` script starts the worker as a helper task and the result comes back
// as a message to the requester.
package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

// quizSeconds is the time members have to answer the qualification quiz; later answers count as failed.
const quizSeconds = 60

// QuizAnswer is a member's private quiz answer (never posted to the message log).
type QuizAnswer struct {
	Answer string    `json:"answer"` // the answer exactly as submitted
	At     Timestamp `json:"at"`     // submission time
	Secs   int       `json:"secs"`   // seconds after the question was asked (> quizSeconds = late)
}

// Quiz is the team's qualification round: the coordinator asks one precise question and
// grades the answers; members who fail are excluded for this task.
type Quiz struct {
	Question string                 `json:"question"` // the question, including the required answer format
	AskedBy  string                 `json:"asked_by"` // the coordinator who asked it
	AskedAt  Timestamp              `json:"asked_at"` // when it was asked (for display)
	Answers  map[string]*QuizAnswer `json:"answers"`  // by member name
}

// BoardItem is a subtask on the team board.
type BoardItem struct {
	ID       string      `json:"id"`                 // "B<n>"
	Title    string      `json:"title"`              // short description shown in board_list
	Details  string      `json:"details"`            // exactly what to do and how to return the result
	Level    BoardLevel  `json:"level"`              // easy (for workers) | normal (for normal models)
	Status   BoardStatus `json:"status"`             // todo | claimed | done
	Creator  string      `json:"creator"`            // receives the result as a message
	Assignee string      `json:"assignee,omitempty"` // who claimed it
	For      string      `json:"for,omitempty"`      // the only member that may claim it ("" = anyone at its level)
	Result   string      `json:"result,omitempty"`   // the result submitted with board_done
	Updated  Timestamp   `json:"updated"`            // last change
}

// Helper is a worker request created with delegate: a normal model hands a simple task to a
// free worker; the `agent` script starts the worker as a helper task on a copy of the
// requester's branch and sends the result back as a message.
type Helper struct {
	ID         int64        `json:"id"`                    // sequential number shown as "H<ID>"
	From       string       `json:"from"`                  // requesting agent
	FromTask   string       `json:"from_task"`             // requesting task
	Repo       string       `json:"repo"`                  // repository name
	Branch     string       `json:"branch"`                // requester's branch (the helper starts from its tip)
	Worktree   string       `json:"worktree"`              // requester's worktree (read-only for the helper)
	Request    string       `json:"request"`               // what to do and in what form to return the result
	Context    string       `json:"context"`               // paths, commands, what to look for
	Status     HelperStatus `json:"status"`                // queued | starting | running | done | failed
	Worker     string       `json:"worker,omitempty"`      // assigned worker agent
	HelperTask string       `json:"helper_task,omitempty"` // task ID of the helper run
	Summary    string       `json:"summary,omitempty"`     // result text sent back to the requester
	Created    Timestamp    `json:"created"`               // when the request was made
}

// agentCmdFn runs an `agent` subcommand in the background; a variable so that tests can intercept it.
var agentCmdFn = spawnAgentCmd

// spawnAgentCmd runs `agent <args>` in the background, detached (like waking). A failure is a side effect:
// the calling operation has already succeeded, so it is logged, not returned.
func spawnAgentCmd(agentCmd, home string, args ...string) {
	if agentCmd == "" {
		warnSideEffect("spawn agent", processErr("agent", nil, "no --agent-cmd configured"), "args", args)
		return
	}
	spawnDetached(home, agentCmd, args...)
}

// spawnDetached starts `program args...` detached from this process (setsid -f when available), with
// AGENTS_HOME set. Start and exit failures are logged as *ProcessError side effects.
func spawnDetached(home, program string, args ...string) {
	cmd := exec.Command(program, args...) //nolint:gosec // G204: the operator configures the local agent command; no shell is involved.
	if _, err := exec.LookPath("setsid"); err == nil {
		cmd = exec.Command("setsid", append([]string{"-f", program}, args...)...) //nolint:gosec // G204: fixed setsid wraps the same operator-configured command.
	}
	cmd.Env = append(os.Environ(), "AGENTS_HOME="+home)
	if err := cmd.Start(); err != nil {
		warnSideEffect("spawn", processErr(program, err, "cannot start %s", program), "args", args)
		return
	}
	go func() {
		if err := cmd.Wait(); err != nil {
			warnSideEffect("spawn", processErr(program, err, "%s exited with an error", program), "args", args)
		}
	}()
}

// leadCheck is the result of requireLead.
type leadCheck int

const (
	leadOK             leadCheck = iota // the caller coordinates its team
	leadNotInTeam                       // the caller is not in a team
	leadNotCoordinator                  // the caller is a member, but another agent (or nobody yet) coordinates
)

// requireLead returns the caller's team and whether the caller is its coordinator. Each operation turns a
// failed check into its own cause.
func (s *Server) requireLead(st *State) (*Team, leadCheck) {
	t := s.myTeam(st)
	if t == nil {
		return nil, leadNotInTeam
	}
	if t.Lead != s.me {
		return t, leadNotCoordinator
	}
	return t, leadOK
}

// notCoordinatorMessage explains who coordinates.
func notCoordinatorMessage(lead string) string {
	if lead == "" {
		lead = "nobody yet"
	}
	return fmt.Sprintf("only the coordinator (%s) can do this", lead)
}

// ---------------------------------------------------------------- qualification quiz

// quizAge returns how many whole seconds ago the quiz question was asked.
func quizAge(q *Quiz) int { return int(time.Since(q.AskedAt.Time).Seconds()) }

// quizStart (MCP quiz_start): the coordinator asks the team its qualification question.
func (s *Server) quizStart(question string) (string, *QuizStartError) {
	question = strings.TrimSpace(question)
	if question == "" {
		return "", opErr(QuizStartMissingQuestion, "%s", missing("question"))
	}
	err := transact(s.bus, QuizStartStorage, func(st *State) *QuizStartError {
		s.touch(st)
		t, check := s.requireLead(st)
		switch check {
		case leadNotInTeam:
			return opErr(QuizStartNotInTeam, "you are not in a team")
		case leadNotCoordinator:
			return opErr(QuizStartNotCoordinator, "%s", notCoordinatorMessage(t.Lead))
		}
		if t.Quiz != nil {
			return opErr(QuizStartAlreadyHeld, "the quiz has already taken place in this team (see quiz_results)")
		}
		if t.Resumed != "" {
			return opErr(QuizStartResumedTeam, "the team has been resumed — members already passed the quiz, skip it")
		}
		t.Quiz = &Quiz{Question: question, AskedBy: s.me, AskedAt: Now(), Answers: map[string]*QuizAnswer{}}
		_, serr := s.bus.post(st, s.me, teamRecipient(t.ID), EventQuizQuestion{Question: question, Seconds: quizSeconds})
		return fromStorage(QuizStartStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("Quiz started. In %d s call quiz_results(wait=true) and grade the answers.", quizSeconds), nil
}

// quizAnswer (MCP quiz_answer): a member submits its final, private answer.
func (s *Server) quizAnswer(answer string) (string, *QuizAnswerError) {
	answer = strings.TrimSpace(answer)
	if answer == "" {
		return "", opErr(QuizAnswerMissingAnswer, "%s", missing("answer"))
	}
	var secs int
	err := transact(s.bus, QuizAnswerStorage, func(st *State) *QuizAnswerError {
		s.touch(st)
		t := s.myTeam(st)
		switch {
		case t == nil:
			return opErr(QuizAnswerNotInTeam, "you are not in a team")
		case t.Quiz == nil:
			return opErr(QuizAnswerNoQuiz, "no quiz in progress — wait for the coordinator's question")
		case t.Lead == s.me:
			return opErr(QuizAnswerCoordinatorAnswers, "the coordinator does not answer the quiz")
		case t.Quiz.Answers[s.me] != nil:
			return opErr(QuizAnswerAlreadyAnswered, "you already answered — the answer is final")
		}
		secs = quizAge(t.Quiz)
		t.Quiz.Answers[s.me] = &QuizAnswer{Answer: answer, At: Now(), Secs: secs}
		return nil
	})
	if err != nil {
		return "", err
	}
	if secs > quizSeconds {
		return fmt.Sprintf("Answer accepted, but LATE (%d s > %d s). Wait for the coordinator's decision.", secs, quizSeconds), nil
	}
	return fmt.Sprintf("Answer accepted (after %d s). Wait for the coordinator's decision (wait_for_message).", secs), nil
}

// quizResults (MCP quiz_results): the coordinator sees all answers with timings; with wait it
// blocks until everyone answered or the time limit passed.
func (s *Server) quizResults(wait bool) (string, *QuizResultsError) {
	for {
		var sb strings.Builder
		complete := false
		err := transact(s.bus, QuizResultsStorage, func(st *State) *QuizResultsError {
			s.touch(st)
			t, check := s.requireLead(st)
			switch check {
			case leadNotInTeam:
				return opErr(QuizResultsNotInTeam, "you are not in a team")
			case leadNotCoordinator:
				return opErr(QuizResultsNotCoordinator, "%s", notCoordinatorMessage(t.Lead))
			}
			if t.Quiz == nil {
				return opErr(QuizResultsNoQuiz, "no quiz has taken place yet (quiz_start)")
			}
			elapsed := quizAge(t.Quiz)
			missing := 0
			fmt.Fprintf(&sb, "Question: %s\nTime since the question was asked: %d s (limit %d s)\n\n", t.Quiz.Question, elapsed, quizSeconds)
			for _, n := range t.names() {
				m := t.Members[n]
				if n == t.Lead || m.State == MemberExcluded {
					continue
				}
				a := t.Quiz.Answers[n]
				if a == nil {
					missing++
					fmt.Fprintf(&sb, "- %s: NO ANSWER\n", n)
					continue
				}
				late := ""
				if a.Secs > quizSeconds {
					late = " (LATE)"
				}
				fmt.Fprintf(&sb, "- %s: %q after %d s%s\n", n, a.Answer, a.Secs, late)
			}
			complete = missing == 0 || elapsed >= quizSeconds
			return nil
		})
		if err != nil {
			return "", err
		}
		if complete || !wait {
			return sb.String() + "\nGrade strictly against your reference answer. No answer or a late answer = fail. " +
				"Exclude everyone who failed: team_exclude(agent, reason).", nil
		}
		time.Sleep(2 * time.Second)
	}
}

// teamExclude (MCP team_exclude): the coordinator excludes a member; its claimed board items return to "todo".
func (s *Server) teamExclude(agent, reason string) (string, *TeamExcludeError) {
	err := transact(s.bus, TeamExcludeStorage, func(st *State) *TeamExcludeError {
		s.touch(st)
		t, check := s.requireLead(st)
		switch check {
		case leadNotInTeam:
			return opErr(TeamExcludeNotInTeam, "you are not in a team")
		case leadNotCoordinator:
			return opErr(TeamExcludeNotCoordinator, "%s", notCoordinatorMessage(t.Lead))
		}
		m := t.Members[agent]
		if m == nil || agent == s.me {
			return opErr(TeamExcludeNotMember, "%s is not a member of your team (or it is you)", agent)
		}
		m.State = MemberExcluded
		t.releaseItemsOf(agent)
		if _, serr := s.bus.post(st, s.me, agent, EventExcludedNotice{Team: t.ID, Reason: reason}); serr != nil {
			return fromStorage(TeamExcludeStorage, serr)
		}
		_, serr := s.bus.post(st, "agentbus", teamRecipient(t.ID), EventMemberExcluded{Member: agent, Reason: reason})
		return fromStorage(TeamExcludeStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return "Excluded " + agent + ".", nil
}

// ---------------------------------------------------------------- task board

// sortedBoardIDs returns the board item IDs in creation order ("B2" before "B10": shorter IDs first).
func (t *Team) sortedBoardIDs() []string {
	ids := make([]string, 0, len(t.Board))
	for id := range t.Board {
		ids = append(ids, id)
	}
	sort.Slice(ids, func(i, j int) bool {
		if len(ids[i]) != len(ids[j]) {
			return len(ids[i]) < len(ids[j])
		}
		return ids[i] < ids[j]
	})
	return ids
}

// releaseItemsOf returns every item claimed by agent to "todo" and returns their IDs.
func (t *Team) releaseItemsOf(agent string) []string {
	var released []string
	for _, id := range t.sortedBoardIDs() {
		it := t.Board[id]
		if it.Assignee == agent && it.Status == BoardClaimed {
			it.Status, it.Assignee, it.Updated = BoardTodo, "", Now()
			released = append(released, id)
		}
	}
	if len(released) > 0 {
		t.bumpBoard()
	}
	return released
}

// tierForLevel returns the tier that should take items of a level.
func tierForLevel(l BoardLevel) Tier {
	if l == LevelEasy {
		return TierWorker
	}
	return TierNormal
}

// wakeOne wakes one sleeping member of the given tier. It balances providers: it picks the member
// from the provider that currently has the fewest working members (tie — alphabetical order).
func (b *Bus) wakeOne(t *Team, tier Tier, except string) {
	busy := map[string]int{}
	for _, m := range t.Members {
		if m.State.Working() {
			busy[m.Provider]++
		}
	}
	var pick *TeamMember
	for _, n := range t.names() {
		m := t.Members[n]
		if n == except || m.State != MemberSleeping || m.Tier != tier {
			continue
		}
		if pick == nil || busy[m.Provider] < busy[pick.Provider] {
			pick = m
		}
	}
	if pick != nil {
		pick.State = MemberWaking
		pick.BoardSeen = t.BoardRev
		wakeFn(t, pick.Task)
	}
}

// wakeDesignated wakes the member an item is designated to, if it sleeps.
func (b *Bus) wakeDesignated(t *Team, name string) {
	if m := t.Members[name]; m != nil && m.State == MemberSleeping {
		m.State = MemberWaking
		m.BoardSeen = t.BoardRev
		wakeFn(t, m.Task)
	}
}

// boardAdd adds an open board item (see boardAddFor).
func (s *Server) boardAdd(title, details, level string) (string, *BoardAddError) {
	return s.boardAddFor(title, details, level, "")
}

// boardAddFor (MCP board_add) adds a board item and wakes one sleeping member who can take it: the designated
// member when forMember is given, otherwise one of the matching tier.
func (s *Server) boardAddFor(title, details, level, forMember string) (string, *BoardAddError) {
	forMember = strings.TrimSpace(forMember)
	title = strings.TrimSpace(title)
	if title == "" {
		return "", opErr(BoardAddMissingTitle, "%s", missing("title"))
	}
	lvl, verr := parseBoardLevel(level)
	if verr != nil {
		return "", fromValue(BoardAddInvalidLevel, verr)
	}
	var id string
	err := transact(s.bus, BoardAddStorage, func(st *State) *BoardAddError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(BoardAddNotInTeam, "the board exists only in a team — outside a team use delegate")
		}
		if forMember != "" {
			m := t.Members[forMember]
			if m == nil || m.State == MemberExcluded {
				return opErr(BoardAddUnknownMember, "%s is not a member of your team (see team_status)", forMember)
			}
			if m.Tier == TierWorker && lvl != LevelEasy {
				return opErr(BoardAddForWorkerNeedsEasy, "%s is a worker: items for workers must be level easy", forMember)
			}
		}
		if t.Board == nil {
			t.Board = map[string]*BoardItem{}
		}
		t.NextBID++
		id = "B" + strconv.Itoa(t.NextBID)
		t.Board[id] = &BoardItem{ID: id, Title: title, Details: strings.TrimSpace(details), Level: lvl, Status: BoardTodo,
			Creator: s.me, For: forMember, Updated: Now()}
		t.bumpBoard()
		if _, serr := s.bus.post(st, s.me, "all", EventBoardItemAdded{ID: id, Level: lvl, Title: title, For: forMember}); serr != nil {
			return fromStorage(BoardAddStorage, serr)
		}
		if forMember != "" {
			s.bus.wakeDesignated(t, forMember)
		} else {
			s.bus.wakeOne(t, tierForLevel(lvl), s.me)
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("Added %s (%s). The result will arrive as a message from whoever does it.", id, lvl), nil
}

// boardList (MCP board_list) renders the team board.
func (s *Server) boardList() (string, *BoardListError) {
	var sb strings.Builder
	err := transact(s.bus, BoardListStorage, func(st *State) *BoardListError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(BoardListNotInTeam, "you are not in a team")
		}
		if me := t.Members[s.me]; me != nil {
			me.BoardSeen = t.BoardRev // the member has now seen every item on the board
		}
		ids := t.sortedBoardIDs()
		if len(ids) == 0 {
			sb.WriteString("The board is empty.\n")
		}
		for _, id := range ids {
			it := t.Board[id]
			who := ""
			if it.Assignee != "" {
				who = " → " + it.Assignee
			}
			if it.For != "" && it.Status == BoardTodo {
				who = " — for " + it.For + " only"
			}
			fmt.Fprintf(&sb, "- %s [%s, %s%s] %s (from %s)\n", id, it.Level, it.Status, who, it.Title, it.Creator)
			if it.Status != BoardDone && it.Details != "" {
				fmt.Fprintf(&sb, "    %s\n", strings.ReplaceAll(it.Details, "\n", "\n    "))
			}
			if it.Status == BoardDone && it.Result != "" {
				fmt.Fprintf(&sb, "    result: %s\n", trunc(it.Result, 300))
			}
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return sb.String(), nil
}

// boardClaim (MCP board_claim) assigns a "todo" item to the caller (workers may claim only easy items).
func (s *Server) boardClaim(id string) (string, *BoardClaimError) {
	var it BoardItem
	err := transact(s.bus, BoardClaimStorage, func(st *State) *BoardClaimError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(BoardClaimNotInTeam, "you are not in a team")
		}
		item := t.Board[strings.TrimSpace(id)]
		if item == nil {
			return opErr(BoardClaimNoSuchItem, "no board item %s (see board_list)", id)
		}
		if item.For != "" && item.For != s.me && item.Status == BoardTodo {
			return opErr(BoardClaimReservedForOther, "%s is reserved for %s", item.ID, item.For)
		}
		if me := t.Members[s.me]; me != nil && me.Tier == TierWorker && item.Level != LevelEasy {
			return opErr(BoardClaimWorkerNeedsEasy, "as a worker you only take easy items")
		}
		if item.Status != BoardTodo {
			return opErr(BoardClaimAlreadyTaken, "%s is already %s (by %s)", item.ID, item.Status, item.Assignee)
		}
		item.Status, item.Assignee, item.Updated = BoardClaimed, s.me, Now()
		it = *item
		return nil
	})
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("You take %s: %s\n%s\nWhen done: board_done(%s, result).", it.ID, it.Title, it.Details, it.ID), nil
}

// boardDone (MCP board_done) stores the result and sends it to the item's creator.
func (s *Server) boardDone(id, result string) (string, *BoardDoneError) {
	result = strings.TrimSpace(result)
	if result == "" {
		return "", opErr(BoardDoneMissingResult, "%s", missing("result"))
	}
	var done string
	err := transact(s.bus, BoardDoneStorage, func(st *State) *BoardDoneError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(BoardDoneNotInTeam, "you are not in a team")
		}
		it := t.Board[strings.TrimSpace(id)]
		if it == nil || it.Assignee != s.me {
			return opErr(BoardDoneNotYourItem, "%s is not your item", id)
		}
		it.Status, it.Result, it.Updated = BoardDone, result, Now()
		done = it.ID
		if it.Creator == s.me {
			return nil
		}
		_, serr := s.bus.post(st, s.me, it.Creator, EventBoardResult{ID: it.ID, Title: it.Title, Result: result})
		return fromStorage(BoardDoneStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return "Result saved for " + done + ". Check board_list before you sleep — there may be something for you.", nil
}

// boardRelease (MCP board_release) returns a claimed item to "todo" with a note to its creator.
func (s *Server) boardRelease(id, note string) (string, *BoardReleaseError) {
	var released string
	err := transact(s.bus, BoardReleaseStorage, func(st *State) *BoardReleaseError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(BoardReleaseNotInTeam, "you are not in a team")
		}
		it := t.Board[strings.TrimSpace(id)]
		if it == nil || it.Assignee != s.me {
			return opErr(BoardReleaseNotYourItem, "%s is not your item", id)
		}
		it.Status, it.Assignee, it.Updated = BoardTodo, "", Now()
		t.bumpBoard()
		if note != "" {
			it.Details += "\n[returned by " + s.me + ": " + note + "]"
		}
		released = it.ID
		_, serr := s.bus.post(st, s.me, it.Creator, EventBoardReturned{ID: it.ID, Title: it.Title, Note: note})
		return fromStorage(BoardReleaseStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return "Returned " + released + " to the board.", nil
}

// ---------------------------------------------------------------- worker requests (delegate)

// delegate (MCP delegate) queues a worker request and spawns the `agent` helper dispatcher.
func (s *Server) delegate(request, context string) (string, *DelegateError) {
	if s.tier == TierWorker || s.role == RoleInvestigator {
		return "", opErr(DelegateNotNormalTier, "delegate is for normal models")
	}
	request = strings.TrimSpace(request)
	if request == "" {
		return "", opErr(DelegateMissingRequest, "%s", missing("request"))
	}
	var id int64
	err := transact(s.bus, DelegateStorage, func(st *State) *DelegateError {
		s.touch(st)
		st.NextHelperID++
		h := &Helper{ID: st.NextHelperID, From: s.me, FromTask: s.info.Task, Repo: s.info.Repo, Branch: s.info.Branch,
			Worktree: s.info.Worktree, Request: request, Context: strings.TrimSpace(context), Status: HelperQueued, Created: Now()}
		st.Helpers[helperKey(h.ID)] = h
		id = h.ID
		_, serr := s.bus.post(st, s.me, "all", EventWorkerRequest{ID: h.ID, Request: request})
		return fromStorage(DelegateStorage, serr)
	})
	if err != nil {
		return "", err
	}
	agentCmdFn(s.agentCmd, s.home, "_dispatch-helpers")
	return fmt.Sprintf("Request H%d accepted. The worker gets the state of your branch's last commit and the path to your worktree "+
		"(read-only). The result will arrive as a message from the worker (wait_for_message) — meanwhile do your own work.", id), nil
}

// delegationStatus (MCP delegation_status) lists the caller's requests and the pending queue.
func (s *Server) delegationStatus() (string, *DelegationStatusError) {
	var sb strings.Builder
	err := transact(s.bus, DelegationStatusStorage, func(st *State) *DelegationStatusError {
		s.touch(st)
		for _, h := range sortedHelpers(st) {
			if h.FromTask != s.info.Task && h.Status != HelperQueued && h.Status != HelperRunning {
				continue
			}
			w := ""
			if h.Worker != "" {
				w = " → " + h.Worker
			}
			fmt.Fprintf(&sb, "- H%d [%s%s] from %s: %s\n", h.ID, h.Status, w, h.From, trunc(h.Request, 120))
			if h.Summary != "" && h.FromTask == s.info.Task {
				fmt.Fprintf(&sb, "    result: %s\n", trunc(h.Summary, 300))
			}
		}
		if sb.Len() == 0 {
			sb.WriteString("No requests.\n")
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return sb.String(), nil
}

// helperKey converts a worker request ID to its State map key.
func helperKey(id int64) string { return strconv.FormatInt(id, 10) }

// sortedHelpers returns worker requests ordered by ID (FIFO).
func sortedHelpers(st *State) []*Helper {
	out := make([]*Helper, 0, len(st.Helpers))
	for _, h := range st.Helpers {
		out = append(out, h)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// collabToolDefs returns the MCP definitions of the quiz/board tools (team only), checkpoint,
// and delegate (normal tier only).
func collabToolDefs(inTeam bool, tier Tier) []map[string]any {
	str := func(d string) map[string]any { return map[string]any{"type": "string", "description": d} }
	obj := func(props map[string]any, req ...string) map[string]any {
		m := map[string]any{"type": "object", "properties": props}
		if len(req) > 0 {
			m["required"] = req
		}
		return m
	}
	tool := func(name, desc string, schema map[string]any) map[string]any {
		return map[string]any{"name": name, "description": desc, "inputSchema": schema}
	}
	var defs []map[string]any
	if inTeam {
		defs = append(defs,
			tool("quiz_start", "Coordinator: ask members one precise qualification question (a single unambiguous answer). Answers are private.",
				obj(map[string]any{"question": str("A question with a clearly specified answer format, e.g. 'reply with a number only'")}, "question")),
			tool("quiz_answer", "Member: answer the quiz question (once, final, within 60 s). The answer only.",
				obj(map[string]any{"answer": str("The answer only")}, "answer")),
			tool("quiz_results", "Coordinator: members' answers with timings. wait=true waits until everyone answered or 60 s passed.",
				obj(map[string]any{"wait": map[string]any{"type": "boolean", "description": "Wait for all answers / the time limit"}})),
			tool("team_exclude", "Coordinator: exclude a member from the team for this task (e.g. failed the quiz).",
				obj(map[string]any{"agent": str("Member"), "reason": str("Reason")}, "agent", "reason")),
			tool("board_add", "Add a subtask to the team board. level=easy: simple, mechanical work for workers (listing, grep, checking "+
				"file contents, running a known command and summarising its output); level=normal: for normal models. Wakes one sleeping member "+
				"who can take it (only the designated member when for is given).",
				obj(map[string]any{"title": str("Short title"), "details": str("Exactly what to do and in what form to return the result"),
					"level": map[string]any{"type": "string", "enum": []string{string(LevelEasy), string(LevelNormal)}},
					"for": str("Optional: the member that must do it. Only that member may claim the item and only it is woken. " +
						"Use this instead of writing \"for X\" or \"wait for assignment\" in the text — an open item is for anyone at its level.")},
					"title", "details", "level")),
			tool("board_list", "Show the team board (todo / in progress / done).", obj(map[string]any{})),
			tool("board_claim", "Take an item from the board (workers: easy only).", obj(map[string]any{"id": str("e.g. B3")}, "id")),
			tool("board_done", "Submit the result of an item — it is sent as a message to whoever requested it.",
				obj(map[string]any{"id": str("e.g. B3"), "result": str("Result")}, "id", "result")),
			tool("board_release", "Return an item to the board (e.g. too hard for you), with a note.",
				obj(map[string]any{"id": str("e.g. B3"), "note": str("Why")}, "id")),
		)
	}
	defs = append(defs, tool("checkpoint", "Save a work checkpoint: what is done (commits, findings, decisions) and what comes next. "+
		"Do it after each completed stage and about every 30 min of work — after an interruption the team resumes from the last checkpoint instead of from scratch. "+
		"Commit and push your branch before the checkpoint.",
		obj(map[string]any{"summary": str("State: what is done, where (commits/files), key findings and decisions"),
			"next_steps": str("What next, in which order, by whom")}, "summary")))
	if tier != TierWorker {
		defs = append(defs,
			tool("delegate", "Delegate a simple, mechanical task to a free worker (a cheap model) instead of doing it yourself and spending expensive tokens: "+
				"listing and searching files, checking whether something exists, running a known command and summarising its output. "+
				"The result will arrive as a message. In a team use the board first (board_add level=easy).",
				obj(map[string]any{"request": str("Exactly what to do and in what form to return the result"), "context": str("Paths, commands, what to look for")}, "request")),
			tool("delegation_status", "Status of your worker requests and of the queue.", obj(map[string]any{})),
		)
	}
	return defs
}

// ---------------------------------------------------------------- CLI used by the `agent` script

// helper-next: the oldest queued request → "starting" (JSON); empty output when none.
func cmdHelperNext(args []string) {
	fs := newFlagSet("helper-next")
	dir := fs.String("dir", defaultDir(), "state directory")
	worker := fs.String("worker", "", "worker that gets it")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdHelperNextStorage, func(st *State) *CmdHelperNextError {
		for _, h := range sortedHelpers(st) {
			if h.Status == HelperQueued {
				h.Status, h.Worker = HelperStarting, *worker
				return fromStorage(CmdHelperNextStorage, printJSON(h))
			}
		}
		return nil
	}).asError())
}

// helper-set: updates a worker request (status, helper task, result summary).
func cmdHelperSet(args []string) {
	fs := newFlagSet("helper-set")
	dir := fs.String("dir", defaultDir(), "state directory")
	id := fs.Int64("id", 0, "request number")
	status := fs.String("status", "", "new status")
	task := fs.String("task", "", "worker task")
	summary := fs.String("summary", "", "result")
	parseFlags(fs, args)
	var newStatus HelperStatus
	if *status != "" {
		v, verr := parseEnum[HelperStatus](*status, "worker request status")
		exitOn(fromValue(CmdHelperSetInvalidStatus, verr).asError())
		newStatus = v
	}
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdHelperSetStorage, func(st *State) *CmdHelperSetError {
		h := st.Helpers[helperKey(*id)]
		if h == nil {
			return opErr(CmdHelperSetNoSuchRequest, "no worker request H%d", *id)
		}
		if newStatus != "" {
			h.Status = newStatus
		}
		if *task != "" {
			h.HelperTask = *task
		}
		if *summary != "" {
			h.Summary = *summary
		}
		return nil
	}).asError())
}

// helper-get: prints a worker request as JSON.
func cmdHelperGet(args []string) {
	fs := newFlagSet("helper-get")
	dir := fs.String("dir", defaultDir(), "state directory")
	id := fs.Int64("id", 0, "request number")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdHelperGetStorage, func(st *State) *CmdHelperGetError {
		h := st.Helpers[helperKey(*id)]
		if h == nil {
			return opErr(CmdHelperGetNoSuchRequest, "no worker request H%d", *id)
		}
		return fromStorage(CmdHelperGetStorage, printJSON(h))
	}).asError())
}

// helper-list: human-readable list of worker requests.
func cmdHelperList(args []string) {
	fs := newFlagSet("helper-list")
	dir := fs.String("dir", defaultDir(), "state directory")
	all := fs.Bool("all", false, "include finished")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdHelperListStorage, func(st *State) *CmdHelperListError {
		n := 0
		for _, h := range sortedHelpers(st) {
			if !*all && !h.Status.Open() {
				continue
			}
			n++
			w := ""
			if h.Worker != "" {
				w = " → " + h.Worker
			}
			out.printf("H%d [%s%s] from %s (%s): %s\n", h.ID, h.Status, w, h.From, h.FromTask, trunc(h.Request, 100))
		}
		if n == 0 {
			out.println("No worker requests.")
		}
		return fromStorage(CmdHelperListStorage, out.err)
	}).asError())
}

// board-dump: the team board as JSON (for `agent live`).
func cmdBoardDump(args []string) {
	fs := newFlagSet("board-dump")
	dir := fs.String("dir", defaultDir(), "state directory")
	team := fs.String("team", "", "team")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdBoardDumpStorage, func(st *State) *CmdBoardDumpError {
		t := st.Teams[*team]
		if t == nil {
			return opErr(CmdBoardDumpNoSuchTeam, "no team %s", *team)
		}
		return fromStorage(CmdBoardDumpStorage, printJSON(map[string]any{"board": t.Board, "quiz": t.Quiz}))
	}).asError())
}

// board-release-agent: items claimed by an agent go back to the board (e.g. a hung worker); prints their IDs.
func cmdBoardReleaseAgent(args []string) {
	fs := newFlagSet("board-release-agent")
	dir := fs.String("dir", defaultDir(), "state directory")
	team := fs.String("team", "", "team")
	agent := fs.String("agent", "", "agent")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdBoardReleaseAgentStorage, func(st *State) *CmdBoardReleaseAgentError {
		t := st.Teams[*team]
		if t == nil {
			return opErr(CmdBoardReleaseAgentNoSuchTeam, "no team %s", *team)
		}
		for _, id := range t.releaseItemsOf(*agent) {
			out.println(id)
		}
		return fromStorage(CmdBoardReleaseAgentStorage, out.err)
	}).asError())
}

// ---------------------------------------------------------------- checkpoints

// checkpointKey returns the checkpoint file key: "team-<id>" in a team, else "task-<id>".
func (s *Server) checkpointKey() string {
	if s.team != "" {
		return "team-" + s.team
	}
	return "task-" + s.info.Task
}

// checkpointPath returns the file of a checkpoint key.
func (b *Bus) checkpointPath(key string) string {
	return filepath.Join(b.dir, "checkpoints", key+".md")
}

// renderCheckpoint renders one checkpoint entry (with the team board when there is one).
func renderCheckpoint(at Timestamp, agent, branch, summary, next string, t *Team) string {
	var sb strings.Builder
	fmt.Fprintf(&sb, "\n## Checkpoint %s — %s (branch %s)\n\n%s\n", at, agent, branch, summary)
	if next != "" {
		fmt.Fprintf(&sb, "\n**Next steps:** %s\n", next)
	}
	if t != nil && len(t.Board) > 0 {
		sb.WriteString("\nBoard right now:\n")
		for _, id := range t.sortedBoardIDs() {
			it := t.Board[id]
			fmt.Fprintf(&sb, "- %s [%s, %s %s] %s\n", id, it.Level, it.Status, it.Assignee, it.Title)
		}
	}
	return sb.String()
}

// checkpoint saves a work state to resume from after an interruption (agent run --resume). The checkpoint
// file is the purpose of this tool, so failing to write it fails the call (CheckpointStorage).
func (s *Server) checkpoint(summary, next string) (string, *CheckpointError) {
	summary = strings.TrimSpace(summary)
	if summary == "" {
		return "", opErr(CheckpointMissingSummary, "%s", missing("summary"))
	}
	err := transact(s.bus, CheckpointStorage, func(st *State) *CheckpointError {
		s.touch(st)
		entry := renderCheckpoint(Now(), s.me, s.info.Branch, summary, strings.TrimSpace(next), s.myTeam(st))
		if serr := appendFile(s.bus.checkpointPath(s.checkpointKey()), entry); serr != nil {
			return fromStorage(CheckpointStorage, serr)
		}
		_, serr := s.bus.post(st, s.me, "all", EventCheckpoint{Summary: summary})
		return fromStorage(CheckpointStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return "Checkpoint saved. Make sure your commits are pushed (git push) — resuming starts from the pushed branches.", nil
}

// checkpoint-get: prints the checkpoint file (used to build the resume prompt); nothing when there is none.
func cmdCheckpointGet(args []string) {
	fs := newFlagSet("checkpoint-get")
	dir := fs.String("dir", defaultDir(), "state directory")
	key := fs.String("key", "", "team-<id> or task-<id>")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	data, err := os.ReadFile(b.checkpointPath(*key))
	if errors.Is(err, os.ErrNotExist) {
		return
	}
	if err != nil {
		exitOn(fromStorage(CmdCheckpointGetStorage, storageErr(err, "cannot read the checkpoint %s", *key)).asError())
	}
	var out cliWriter
	out.printf("%s", data)
	exitOn(fromStorage(CmdCheckpointGetStorage, out.err).asError())
}
