// Teams, sleeping/waking, wait_for_message and the repository notebook.
//
// Team: several agents with the same task (each in its own worktree/branch).
// They elect the coordinator themselves (claim_lead / handover_lead). A member with no
// work ends its run with STATUS: SLEEPING; the `agent` script marks it "sleeping",
// and every message addressed to it (or to "team") runs `agent _wake <task>`.
//
// Notebook: one per repository, persistent across tasks — an append-only log
// plus named sections that can be overwritten.
package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// TeamMember is one agent of a team.
type TeamMember struct {
	Agent    string      `json:"agent"`              // agent name
	Task     string      `json:"task"`               // the member's task ID
	Branch   string      `json:"branch"`             // the member's git branch
	Worktree string      `json:"worktree"`           // the member's worktree (teammates may read it)
	State    MemberState `json:"state"`              // active | sleeping | waking | done | excluded | hung
	Tier     Tier        `json:"tier"`               // normal | worker
	Provider string      `json:"provider,omitempty"` // anthropic | openai | google | openrouter | deepinfra | ... (load balancing)
}

// Team is a group of agents working on the same task, each on its own branch. The
// coordinator (Lead) is elected by the members via claim_lead (normal tier only).
type Team struct {
	ID      string                 `json:"id"`                     // "<timestamp>-team"
	Repo    string                 `json:"repo"`                   // repository name
	Lead    string                 `json:"lead"`                   // coordinator's agent name ("" until elected)
	WakeCmd string                 `json:"wake_cmd"`               // `agent` script used to wake sleeping members
	Home    string                 `json:"home"`                   // AGENTS_HOME for spawned wake processes
	Done    bool                   `json:"done"`                   // set when the team's task is finished or stopped
	Created Timestamp              `json:"created"`                // when the team was created
	Members map[string]*TeamMember `json:"members"`                // by agent name
	Quiz    *Quiz                  `json:"quiz,omitempty"`         // the qualification quiz (nil until it is asked)
	Board   map[string]*BoardItem  `json:"board,omitempty"`        // by item ID ("B1", "B2", ...)
	NextBID int                    `json:"next_board_id"`          // last board item number issued
	Resumed string                 `json:"resumed_from,omitempty"` // previous team ID when created by agent run --resume
}

// names returns the member names sorted alphabetically.
func (t *Team) names() []string {
	out := make([]string, 0, len(t.Members))
	for n := range t.Members {
		out = append(out, n)
	}
	sort.Strings(out)
	return out
}

// wakeFn wakes a sleeping member; a variable so that tests can observe wake-ups without spawning processes.
var wakeFn = spawnWake

// spawnWake runs `agent _wake <task>` in the background (detached from the current process). Failures are
// logged: the message that triggered the wake-up has been stored and stays unread for the member.
func spawnWake(t *Team, task string) {
	if t.WakeCmd == "" {
		warnSideEffect("wake member", processErr("agent", nil, "team %s has no wake command", t.ID), "task", task)
		return
	}
	logInfo(LogMemberWaking, "team", t.ID, "task", task)
	spawnDetached(t.Home, t.WakeCmd, "_wake", task)
}

// wakeFor wakes sleeping recipients after a message is appended (direct or "team:<id>").
func (b *Bus) wakeFor(st *State, m Message) {
	for _, t := range st.Teams {
		if t.Done {
			continue
		}
		for name, mem := range t.Members {
			if mem.State != MemberSleeping || name == m.From {
				continue
			}
			if m.To == name || m.To == teamRecipient(t.ID) {
				mem.State = MemberWaking
				wakeFn(t, mem.Task)
			}
		}
	}
}

// isForAgent reports whether a message is for the agent (direct, to its team, or — optionally — to all).
func isForAgent(st *State, m Message, agent string, broadcast bool) bool {
	if m.From == agent {
		return false
	}
	if m.To == agent {
		return true
	}
	if strings.HasPrefix(m.To, "team:") {
		t := st.Teams[strings.TrimPrefix(m.To, "team:")]
		return t != nil && t.Members[agent] != nil
	}
	return broadcast && m.To == "all"
}

// ---------------------------------------------------------------- MCP tools: team

// myTeam returns this agent's team (nil outside team mode).
func (s *Server) myTeam(st *State) *Team {
	if s.team == "" {
		return nil
	}
	return st.Teams[s.team]
}

// teamStatus (MCP team_status) shows members, their state, branches and the board summary.
func (s *Server) teamStatus() (string, *TeamStatusError) {
	var sb strings.Builder
	err := transact(s.bus, TeamStatusStorage, func(st *State) *TeamStatusError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(TeamStatusNotInTeam, "you are not in a team")
		}
		lead := t.Lead
		if lead == "" {
			lead = "(not chosen yet — claim_lead)"
		}
		fmt.Fprintf(&sb, "Team %s (repo %s), coordinator: %s%s\n", t.ID, t.Repo, lead, map[bool]string{true: ", FINISHED", false: ""}[t.Done])
		for _, n := range t.names() {
			m := t.Members[n]
			you := ""
			if n == s.me {
				you = " [you]"
			}
			stage := ""
			if p := st.Agents[n]; p != nil && p.Task == m.Task && p.Status != "" {
				stage = ", stage: " + p.Status
			}
			fmt.Fprintf(&sb, "- %s%s [%s]: %s, branch %s, worktree %s%s\n", n, you, m.Tier, m.State, m.Branch, m.Worktree, stage)
		}
		if len(t.Board) > 0 {
			todo, claimed, done := 0, 0, 0
			for _, it := range t.Board {
				switch it.Status {
				case BoardTodo:
					todo++
				case BoardClaimed:
					claimed++
				case BoardDone:
					done++
				}
			}
			fmt.Fprintf(&sb, "Board: todo %d, in progress %d, done %d (board_list)\n", todo, claimed, done)
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return sb.String(), nil
}

// claimLead (MCP claim_lead): the first normal-tier member to call it becomes the coordinator;
// a hung coordinator can be replaced. A worker gets an explanation (a result, not an error).
func (s *Server) claimLead() (string, *ClaimLeadError) {
	var msg string
	err := transact(s.bus, ClaimLeadStorage, func(st *State) *ClaimLeadError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(ClaimLeadNotInTeam, "you are not in a team")
		}
		if m := t.Members[s.me]; m != nil && m.Tier == TierWorker {
			msg = "You are a worker — workers cannot coordinate. Only models from the normal list can be coordinators."
			if t.Lead != "" {
				msg += " Coordinator: " + t.Lead + "."
			}
			return nil
		}
		if t.Lead != "" && t.Lead != s.me && t.Members[t.Lead] != nil && t.Members[t.Lead].State == MemberHung {
			old := t.Lead
			t.Lead = s.me
			msg = "You are the coordinator of team " + t.ID + " (taken over from hung " + old + ")."
			_, serr := s.bus.post(st, s.me, teamRecipient(t.ID), EventCoordinatorTakeover{Coordinator: s.me, Previous: old})
			return fromStorage(ClaimLeadStorage, serr)
		}
		switch t.Lead {
		case "":
			t.Lead = s.me
			msg = "You are the coordinator of team " + t.ID + "."
			_, serr := s.bus.post(st, s.me, teamRecipient(t.ID), EventCoordinatorClaimed{Coordinator: s.me})
			return fromStorage(ClaimLeadStorage, serr)
		case s.me:
			msg = "You are already the coordinator."
		default:
			msg = "The coordinator is " + t.Lead + ". Wait for your assignment (sleep if you have nothing to do)."
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return msg, nil
}

// handoverLead (MCP handover_lead): the coordinator hands the role to another normal member.
func (s *Server) handoverLead(to string) (string, *HandoverLeadError) {
	err := transact(s.bus, HandoverLeadStorage, func(st *State) *HandoverLeadError {
		s.touch(st)
		t, check := s.requireLead(st)
		switch check {
		case leadNotInTeam:
			return opErr(HandoverLeadNotInTeam, "you are not in a team")
		case leadNotCoordinator:
			return opErr(HandoverLeadNotCoordinator, "only the coordinator (%s) can hand over the role", t.Lead)
		}
		target := t.Members[to]
		switch {
		case target == nil:
			return opErr(HandoverLeadNotMember, "%s is not a member of your team", to)
		case target.Tier == TierWorker:
			return opErr(HandoverLeadTargetIsWorker, "%s is a worker — workers cannot coordinate", to)
		case target.State == MemberExcluded:
			return opErr(HandoverLeadTargetExcluded, "%s is excluded from the team", to)
		}
		t.Lead = to
		_, serr := s.bus.post(st, s.me, teamRecipient(t.ID), EventCoordinatorHandover{From: s.me, To: to})
		return fromStorage(HandoverLeadStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return "Coordination handed over: " + to, nil
}

// waitForMessage waits (up to timeout s) for a message to me / my team (optionally from a specific
// sender). When one arrives, returns all unread messages and marks them read.
func (s *Server) waitForMessage(from string, timeout int, broadcast bool) (string, *WaitForMessageError) {
	if timeout <= 0 {
		timeout = 300
	}
	if timeout > maxWaitSeconds {
		timeout = maxWaitSeconds
	}
	from = strings.TrimSpace(from)
	deadline := time.Now().Add(time.Duration(timeout) * time.Second)
	for {
		var sb strings.Builder
		found := false
		err := transact(s.bus, WaitForMessageStorage, func(st *State) *WaitForMessageError {
			s.touch(st)
			msgs, serr := s.bus.messages()
			if serr != nil {
				return fromStorage(WaitForMessageStorage, serr)
			}
			cur := st.Cursors[s.me]
			for _, m := range msgs {
				if m.ID > cur && isForAgent(st, m, s.me, broadcast) && (from == "" || m.From == from) {
					found = true
					break
				}
			}
			if !found {
				return nil
			}
			for _, m := range msgs {
				if m.ID > cur && isForAgent(st, m, s.me, true) {
					sb.WriteString(fmtMsg(m) + "\n")
				}
			}
			st.Cursors[s.me] = st.NextID - 1
			return nil
		})
		if err != nil {
			return "", err
		}
		if found {
			return sb.String(), nil
		}
		if !time.Now().Before(deadline) {
			return fmt.Sprintf("No new messages after %d s. You can wait again or — if you have nothing to do — "+
				"end your turn with STATUS: SLEEPING (a message will wake you up).", timeout), nil
		}
		time.Sleep(2 * time.Second)
	}
}

// ---------------------------------------------------------------- repository notebook

// NoteSection is a named, overwritable section of a repository notebook.
type NoteSection struct {
	Text     string    `json:"text"`               // the section content (markdown)
	Author   string    `json:"author"`             // agent that wrote the current text
	Task     string    `json:"task"`               // task in which it was written
	Updated  Timestamp `json:"updated"`            // when it was written
	Previous string    `json:"previous,omitempty"` // previous text (one level of undo)
}

// NoteEntry is one entry of a repository notebook's append-only log.
type NoteEntry struct {
	TS     Timestamp `json:"ts"`            // when the entry was added
	Author string    `json:"author"`        // agent that added it
	Task   string    `json:"task"`          // task in which it was added
	Tag    string    `json:"tag,omitempty"` // optional topic, e.g. "hardware", "toolchain"
	Text   string    `json:"text"`          // the finding itself
}

// safeName maps a repository name to a safe directory name.
func safeName(s string) string {
	s = strings.TrimSpace(s)
	var b strings.Builder
	for _, r := range s {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9', r == '-', r == '_', r == '.':
			b.WriteRune(r)
		default:
			b.WriteRune('_')
		}
	}
	if b.Len() == 0 {
		return "_"
	}
	return b.String()
}

// notebookDir returns the notebook directory of a repository.
func (b *Bus) notebookDir(repo string) string {
	return filepath.Join(b.dir, "notebooks", safeName(repo))
}

// loadSections reads the notebook's named sections (none yet = an empty map).
func (b *Bus) loadSections(repo string) (map[string]*NoteSection, *StorageError) {
	secs := map[string]*NoteSection{}
	path := filepath.Join(b.notebookDir(repo), "sections.json")
	data, err := os.ReadFile(path) //nolint:gosec // path is the notebook file under AGENTS_HOME, which only the operator controls
	if errors.Is(err, os.ErrNotExist) {
		return secs, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read %s", path)
	}
	if err := json.Unmarshal(data, &secs); err != nil {
		return nil, storageErr(err, "cannot decode %s", path)
	}
	return secs, nil
}

// saveSections writes the notebook's sections atomically.
func (b *Bus) saveSections(repo string, secs map[string]*NoteSection) *StorageError {
	dir := b.notebookDir(repo)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return storageErr(err, "cannot create the notebook directory of %s", repo)
	}
	data, err := json.MarshalIndent(secs, "", "  ")
	if err != nil {
		return storageErr(err, "cannot encode the notebook sections of %s", repo)
	}
	tmp := filepath.Join(dir, "sections.json.tmp")
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return storageErr(err, "cannot write the notebook sections of %s", repo)
	}
	if err := os.Rename(tmp, filepath.Join(dir, "sections.json")); err != nil {
		return storageErr(err, "cannot replace the notebook sections of %s", repo)
	}
	return nil
}

// appendNote appends an entry to the notebook log.
func (b *Bus) appendNote(repo string, e NoteEntry) *StorageError {
	line, err := json.Marshal(e)
	if err != nil {
		return storageErr(err, "cannot encode a notebook entry of %s", repo)
	}
	return appendFile(filepath.Join(b.notebookDir(repo), "log.jsonl"), string(line)+"\n")
}

// readNotes reads the whole notebook log (none yet = no entries). Corrupt lines are skipped and reported once.
func (b *Bus) readNotes(repo string) ([]NoteEntry, *StorageError) {
	path := filepath.Join(b.notebookDir(repo), "log.jsonl")
	f, err := os.Open(path) //nolint:gosec // path is under AGENTS_HOME, which only the operator controls
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read %s", path)
	}
	defer func() {
		if err := f.Close(); err != nil {
			warnSideEffect("close notebook log", storageErr(err, "cannot close %s", path))
		}
	}()
	var out []NoteEntry
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 1<<20), 1<<24)
	for lineNo := 1; sc.Scan(); lineNo++ {
		var e NoteEntry
		if err := json.Unmarshal(sc.Bytes(), &e); err != nil {
			warnOnce(fmt.Sprintf("%s:%d", path, lineNo), LogCorruptNoteLine,
				storageErr(err, "cannot decode line %d of %s", lineNo, path))
			continue
		}
		out = append(out, e)
	}
	if err := sc.Err(); err != nil {
		return nil, storageErr(err, "cannot read %s", path)
	}
	return out, nil
}

// fmtNote renders a log entry as a single line.
func fmtNote(e NoteEntry) string {
	tag := ""
	if e.Tag != "" {
		tag = " [" + e.Tag + "]"
	}
	return fmt.Sprintf("(%s %s%s) %s", e.TS, e.Author, tag, e.Text)
}

// renderNotebook renders the notebook (for agents and humans).
func (b *Bus) renderNotebook(repo, section string, last int, search string) (string, *StorageError) {
	var sb strings.Builder
	secs, err := b.loadSections(repo)
	if err != nil {
		return "", err
	}
	if section != "" {
		sec := secs[section]
		if sec == nil {
			return fmt.Sprintf("No section %q. Existing: %s\n", section, strings.Join(sortedKeys(secs), ", ")), nil
		}
		return fmt.Sprintf("## %s  (last change: %s, %s)\n\n%s\n", section, sec.Author, sec.Updated, sec.Text), nil
	}
	notes, err := b.readNotes(repo)
	if err != nil {
		return "", err
	}
	if search != "" {
		q := strings.ToLower(search)
		fmt.Fprintf(&sb, "Search results for %q in the repository notebook %s:\n", search, repo)
		for _, name := range sortedKeys(secs) {
			for _, line := range strings.Split(secs[name].Text, "\n") {
				if strings.Contains(strings.ToLower(line), q) {
					fmt.Fprintf(&sb, "- section %s: %s\n", name, strings.TrimSpace(line))
				}
			}
		}
		for _, e := range notes {
			if strings.Contains(strings.ToLower(e.Text+" "+e.Tag), q) {
				sb.WriteString("- " + fmtNote(e) + "\n")
			}
		}
		return sb.String(), nil
	}
	if last <= 0 {
		last = 15
	}
	if last > 200 {
		last = 200
	}
	fmt.Fprintf(&sb, "Repository notebook %s\n\nSections (full text: notebook_read with section):\n", repo)
	if len(secs) == 0 {
		sb.WriteString("  (none)\n")
	}
	for _, name := range sortedKeys(secs) {
		s := secs[name]
		fmt.Fprintf(&sb, "- %s — %s (%s, %s)\n", name, trunc(firstNonEmpty(s.Text), 100), s.Author, s.Updated)
	}
	start := len(notes) - last
	if start < 0 {
		start = 0
	}
	fmt.Fprintf(&sb, "\nLog (last %d of %d entries):\n", len(notes)-start, len(notes))
	for _, e := range notes[start:] {
		sb.WriteString("- " + fmtNote(e) + "\n")
	}
	return sb.String(), nil
}

// firstNonEmpty returns the first non-blank line.
func firstNonEmpty(s string) string {
	for _, l := range strings.Split(s, "\n") {
		if strings.TrimSpace(l) != "" {
			return l
		}
	}
	return ""
}

// sortedKeys returns section names sorted alphabetically.
func sortedKeys(m map[string]*NoteSection) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// notebookRead (MCP notebook_read).
func (s *Server) notebookRead(section string, last int, search string) (string, *NotebookReadError) {
	var out string
	err := transact(s.bus, NotebookReadStorage, func(st *State) *NotebookReadError {
		s.touch(st)
		var serr *StorageError
		out, serr = s.bus.renderNotebook(s.info.Repo, strings.TrimSpace(section), last, strings.TrimSpace(search))
		return fromStorage(NotebookReadStorage, serr)
	})
	if err != nil {
		return "", err
	}
	return out, nil
}

// notebookAppend (MCP notebook_append).
func (s *Server) notebookAppend(text, tag string) (string, *NotebookAppendError) {
	text = strings.TrimSpace(text)
	if text == "" {
		return "", opErr(NotebookAppendMissingText, "%s", missing("text"))
	}
	err := transact(s.bus, NotebookAppendStorage, func(st *State) *NotebookAppendError {
		s.touch(st)
		entry := NoteEntry{TS: Now(), Author: s.me, Task: s.info.Task, Tag: strings.TrimSpace(tag), Text: text}
		return fromStorage(NotebookAppendStorage, s.bus.appendNote(s.info.Repo, entry))
	})
	if err != nil {
		return "", err
	}
	return "Appended to the repository notebook " + s.info.Repo + ".", nil
}

// notebookWriteSection (MCP notebook_write_section): overwrite, create or (empty text) delete a section;
// the previous text is kept and the change is logged.
func (s *Server) notebookWriteSection(name, text string) (string, *NotebookWriteSectionError) {
	name = strings.TrimSpace(name)
	if name == "" {
		return "", opErr(NotebookWriteSectionMissingName, "%s", missing("name"))
	}
	var msg string
	err := transact(s.bus, NotebookWriteSectionStorage, func(st *State) *NotebookWriteSectionError {
		s.touch(st)
		secs, serr := s.bus.loadSections(s.info.Repo)
		if serr != nil {
			return fromStorage(NotebookWriteSectionStorage, serr)
		}
		old := secs[name]
		note := NoteEntry{TS: Now(), Author: s.me, Task: s.info.Task, Tag: "section"}
		if strings.TrimSpace(text) == "" {
			if old == nil {
				msg = "The section did not exist."
				return nil
			}
			delete(secs, name)
			note.Text = fmt.Sprintf("deleted section %q (had %d chars)", name, len(old.Text))
			msg = "Deleted section " + name + "."
		} else {
			sec := &NoteSection{Text: strings.TrimSpace(text), Author: s.me, Task: s.info.Task, Updated: Now()}
			if old != nil {
				sec.Previous = old.Text
				note.Text = fmt.Sprintf("overwrote section %q (was %d chars, now %d)", name, len(old.Text), len(sec.Text))
			} else {
				note.Text = fmt.Sprintf("created section %q", name)
			}
			secs[name] = sec
			msg = "Saved section " + name + "."
		}
		if serr := s.bus.saveSections(s.info.Repo, secs); serr != nil {
			return fromStorage(NotebookWriteSectionStorage, serr)
		}
		// the section itself is saved; the log entry about the change is auxiliary
		if serr := s.bus.appendNote(s.info.Repo, note); serr != nil {
			warnSideEffect("log section change", serr, "section", name)
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return msg, nil
}

// teamToolDefs returns wait_for_message and notebook tools (everyone) plus team tools (team mode).
func teamToolDefs(inTeam bool) []map[string]any {
	str := func(d string) map[string]any { return map[string]any{"type": "string", "description": d} }
	num := func(d string) map[string]any { return map[string]any{"type": "integer", "description": d} }
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
	defs := []map[string]any{
		tool("wait_for_message", "Wait (blocking, max 600 s) for a message to you or your team — e.g. a teammate's reply. "+
			"When one arrives, returns all unread messages. With 'from' it waits only for that sender.",
			obj(map[string]any{
				"from":              str("Optional: wait only for a message from this agent"),
				"timeout_seconds":   num("Maximum wait in seconds (default 300, max 600)"),
				"include_broadcast": map[string]any{"type": "boolean", "description": "Whether messages to 'all' also end the wait (default: no)"},
			})),
		tool("notebook_read", "Read the persistent repository notebook (shared by all agents and tasks): section list + latest log entries, "+
			"a full section (section) or a search (search).",
			obj(map[string]any{"section": str("Name of the section to read in full"), "last": num("How many latest log entries (default 15)"), "search": str("Search for a phrase in sections and in the log")})),
		tool("notebook_append", "Append a persistent entry to the repository notebook log (findings, pitfalls, tool/hardware behaviour — things useful in future tasks).",
			obj(map[string]any{"text": str("Entry text — concise and specific"), "tag": str("Optional tag, e.g. hardware, toolchain, conventions")}, "text")),
		tool("notebook_write_section", "Overwrite (or create) a named section of the repository notebook, e.g. 'hypotheses', 'verified', 'work split <team>'. Empty text deletes the section.",
			obj(map[string]any{"name": str("Section name"), "text": str("Full new section text (markdown)")}, "name", "text")),
	}
	if inTeam {
		defs = append(defs,
			tool("team_status", "Show the team: members, their state (active/sleeping/done), branches, worktrees, stage and the coordinator.", obj(map[string]any{})),
			tool("claim_lead", "Volunteer as team coordinator. The first one wins; the others learn who coordinates.", obj(map[string]any{})),
			tool("handover_lead", "The coordinator hands the role over to another team member.", obj(map[string]any{"to": str("Member name")}, "to")),
		)
	}
	return defs
}

// ---------------------------------------------------------------- CLI used by the `agent` script

// team-create: team JSON on stdin.
func cmdTeamCreate(args []string) {
	fs := newFlagSet("team-create")
	dir := fs.String("dir", defaultDir(), "state directory")
	parseFlags(fs, args)
	var t Team
	if err := json.NewDecoder(os.Stdin).Decode(&t); err != nil {
		exitOn(&CmdTeamCreateError{Cause: CmdTeamCreateInvalidJSON, Message: "cannot decode the team JSON on stdin: " + err.Error(),
			Err: errors.Join(ErrInvalidArgument, err)})
	}
	if t.ID == "" || len(t.Members) == 0 {
		exitOn(opErr(CmdTeamCreateMissingFields, "the team JSON needs an id and members").asError())
	}
	t.Created = Now()
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdTeamCreateStorage, func(st *State) *CmdTeamCreateError {
		st.Teams[t.ID] = &t
		var announcement busEvent = EventTeamCreated{Team: t.ID, Members: t.names()}
		if t.Resumed != "" {
			announcement = EventTeamResumed{Team: t.ID, Previous: t.Resumed, Members: t.names(), Lead: t.Lead}
		}
		_, serr := b.post(st, "agentbus", teamRecipient(t.ID), announcement)
		return fromStorage(CmdTeamCreateStorage, serr)
	}).asError())
}

// team-set changes a member's state; for "sleeping" it wakes the member right away if messages or board items wait for it.
func cmdTeamSet(args []string) {
	fs := newFlagSet("team-set")
	dir := fs.String("dir", defaultDir(), "state directory")
	team := fs.String("team", "", "team ID")
	agent := fs.String("agent", "", "member")
	state := fs.String("state", "", "the member's new state (active | sleeping | done | excluded | hung)")
	parseFlags(fs, args)
	newState, verr := parseEnum[MemberState](*state, "member state")
	exitOn(fromValue(CmdTeamSetInvalidState, verr).asError())
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdTeamSetStorage, func(st *State) *CmdTeamSetError {
		t := st.Teams[*team]
		if t == nil || t.Members[*agent] == nil {
			return opErr(CmdTeamSetNoSuchMember, "no member %s in team %s", *agent, *team)
		}
		m := t.Members[*agent]
		if m.State == MemberExcluded && newState != MemberDone {
			return nil // exclusion is final
		}
		m.State = newState
		if newState != MemberSleeping || t.Done {
			return nil
		}
		msgs, serr := b.messages()
		if serr != nil {
			return fromStorage(CmdTeamSetStorage, serr)
		}
		cur := st.Cursors[*agent]
		for _, msg := range msgs {
			if msg.ID > cur && isForAgent(st, msg, *agent, false) {
				m.State = MemberWaking
				wakeFn(t, m.Task)
				out.println("waking")
				return fromStorage(CmdTeamSetStorage, out.err)
			}
		}
		// something at this member's level is waiting on the board (it may have been added while it was finishing)
		want := LevelNormal
		if m.Tier == TierWorker {
			want = LevelEasy
		}
		for _, it := range t.Board {
			if it.Status == BoardTodo && it.Level == want {
				m.State = MemberWaking
				wakeFn(t, m.Task)
				out.println("waking (board)")
				return fromStorage(CmdTeamSetStorage, out.err)
			}
		}
		return nil
	}).asError())
}

// team-done: marks a team finished (nothing to do for an unknown or already finished team).
func cmdTeamDone(args []string) {
	fs := newFlagSet("team-done")
	dir := fs.String("dir", defaultDir(), "state directory")
	team := fs.String("team", "", "team ID")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdTeamDoneStorage, func(st *State) *CmdTeamDoneError {
		t := st.Teams[*team]
		if t == nil || t.Done {
			return nil
		}
		t.Done = true
		_, serr := b.post(st, "agentbus", teamRecipient(t.ID), EventTeamFinished{Team: t.ID})
		return fromStorage(CmdTeamDoneStorage, serr)
	}).asError())
}

// team-get: prints a team as JSON (exit status 3 = no such team, see ErrorClass.ExitCode).
func cmdTeamGet(args []string) {
	fs := newFlagSet("team-get")
	dir := fs.String("dir", defaultDir(), "state directory")
	team := fs.String("team", "", "team ID")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdTeamGetStorage, func(st *State) *CmdTeamGetError {
		t := st.Teams[*team]
		if t == nil {
			return opErr(CmdTeamGetNoSuchTeam, "no team %s", *team)
		}
		return fromStorage(CmdTeamGetStorage, printJSON(t))
	}).asError())
}

// team-list: human-readable list of teams.
func cmdTeamList(args []string) {
	fs := newFlagSet("team-list")
	dir := fs.String("dir", defaultDir(), "state directory")
	all := fs.Bool("all", false, "include finished")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdTeamListStorage, func(st *State) *CmdTeamListError {
		ids := make([]string, 0, len(st.Teams))
		for id := range st.Teams {
			ids = append(ids, id)
		}
		sort.Strings(ids)
		for _, id := range ids {
			t := st.Teams[id]
			if t.Done && !*all {
				continue
			}
			state := "active"
			if t.Done {
				state = "finished"
			}
			lead := t.Lead
			if lead == "" {
				lead = "—"
			}
			out.printf("%s [%s] repo %s, coordinator %s\n", t.ID, state, t.Repo, lead)
			for _, n := range t.names() {
				m := t.Members[n]
				out.printf("  %-12s %-9s %s\n", n, m.State, m.Task)
			}
		}
		return fromStorage(CmdTeamListStorage, out.err)
	}).asError())
}

// notes: prints a repository notebook (or lists notebooks).
func cmdNotes(args []string) {
	fs := newFlagSet("notes")
	dir := fs.String("dir", defaultDir(), "state directory")
	repo := fs.String("repo", "", "repository name")
	section := fs.String("section", "", "section to show")
	last := fs.Int("n", 30, "how many latest log entries")
	search := fs.String("search", "", "phrase to search for")
	list := fs.Bool("list", false, "list repositories that have notebooks")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	if *list || *repo == "" {
		entries, err := os.ReadDir(filepath.Join(*dir, "notebooks"))
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			exitOn(fromStorage(CmdNotesStorage, storageErr(err, "cannot list the notebooks")).asError())
		}
		var out cliWriter
		for _, e := range entries {
			if e.IsDir() {
				out.println(e.Name())
			}
		}
		exitOn(fromStorage(CmdNotesStorage, out.err).asError())
		return
	}
	text, serr := b.renderNotebook(*repo, *section, *last, *search)
	exitOn(fromStorage(CmdNotesStorage, serr).asError())
	var out cliWriter
	out.printf("%s", text)
	exitOn(fromStorage(CmdNotesStorage, out.err).asError())
}
