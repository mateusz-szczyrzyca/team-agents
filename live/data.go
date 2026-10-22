package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

// ---------------------------------------------------------------- data model

// Activity is one entry of a task's activity.jsonl, written by `agentbus normalize` and the agent script.
type Activity struct {
	TS   string `json:"ts"` // "2006-01-02 15:04:05" (older entries: "15:04:05")
	Kind string `json:"k"`  // text | cmd | edit | tool | mcp | err | info
	Text string `json:"t"`  // human-readable summary of the step
}

// Task is one task directory (~/agents/tasks/<id>) as seen by the live view, built from its meta.env.
type Task struct {
	ID         string    // task ID: "<timestamp>-<agent>" (investigations/helpers have their own suffix)
	Agent      string    // agent name
	Kind       string    // worker (a normal task) | investigation | helper (a worker request)
	Type       string    // agent CLI: claude | codex | gemini | opencode
	Model      string    // model as configured ("" = the CLI's default)
	Repo       string    // repository name
	Branch     string    // the task's git branch
	Worktree   string    // the task's git worktree
	Status     string    // new | starting | running | waiting | investigating | sleeping | done | failed | stopped | excluded | hung
	InvID      string    // investigations: the investigation number
	ParentTask string    // investigations/helpers: the task they work for
	FromAgent  string    // investigations/helpers: the requesting agent
	Team       string    // team ID ("" for a solo task)
	HelperID   string    // helpers: the worker request number
	Tier       string    // normal | worker
	Round      int       // question rounds so far (questions-<Round>.md is the latest)
	Run        int       // runs so far (prompt-<Run>.md is the latest)
	Created    time.Time // when the task was created
	Finished   time.Time // when the last run ended (zero while running)
	RunStart   time.Time // when the current run started (from the runner process)
	Dir        string    // task directory
	Alive      bool      // the runner process is alive
	Acked      bool      // a finished task was acknowledged (agent ack / x) and is hidden
	Last       Activity  // latest activity entry
}

// Presence is the part of agentbus' presence entry that the live view needs (see agentbus Presence).
type Presence struct {
	Agent  string `json:"agent"`  // agent name
	Task   string `json:"task"`   // task the agent works on
	Status string `json:"status"` // free-form stage text set by the agent (set_status)
}

// Investigation is the part of an agentbus investigation record that the live view needs.
type Investigation struct {
	ID        int64  `json:"id"`         // investigation number
	FromAgent string `json:"from_agent"` // requesting agent
	FromTask  string `json:"from_task"`  // requesting task
	Question  string `json:"question"`   // the question asked
	Status    string `json:"status"`     // queued | waitroom | starting | running | waiting_human | done | failed | rejected
	InvTask   string `json:"inv_task"`   // task ID of the investigator run
	File      string `json:"file"`       // markdown record of the investigation
	Summary   string `json:"summary"`    // one-line result summary
	Decision  string `json:"decision"`   // the requester's decision after the result
}

// Message is one bus message (a line of messages.jsonl).
type Message struct {
	ID   int64  `json:"id"`   // sequential message ID
	TS   string `json:"ts"`   // local time
	From string `json:"from"` // sender
	To   string `json:"to"`   // agent name, "all" or "team:<id>"
	Text string `json:"text"` // message body
}

// BoardItem is a team board item (see agentbus BoardItem).
type BoardItem struct {
	ID       string `json:"id"`       // "B<n>"
	Title    string `json:"title"`    // short description
	Details  string `json:"details"`  // what to do and how to return the result
	Level    string `json:"level"`    // easy | normal
	Status   string `json:"status"`   // todo | claimed | done
	Creator  string `json:"creator"`  // who added it (receives the result)
	Assignee string `json:"assignee"` // who claimed it
	Result   string `json:"result"`   // the submitted result
}

// QuizAnswerInfo is one member's quiz answer.
type QuizAnswerInfo struct {
	Answer string `json:"answer"` // the answer as submitted
	Secs   int    `json:"secs"`   // seconds after the question was asked
}

// QuizInfo is a team's qualification quiz.
type QuizInfo struct {
	Question string                     `json:"question"` // the question
	AskedAt  string                     `json:"asked_at"` // when it was asked
	Answers  map[string]*QuizAnswerInfo `json:"answers"`  // answers by member name
}

// MemberInfo is a team member's state.
type MemberInfo struct {
	State string `json:"state"` // active | sleeping | waking | done | excluded | hung
	Tier  string `json:"tier"`  // normal | worker
}

// TeamInfo is the part of an agentbus team that the live view needs.
type TeamInfo struct {
	ID      string                 `json:"id"`      // team ID
	Lead    string                 `json:"lead"`    // coordinator's agent name
	Done    bool                   `json:"done"`    // the team task is finished
	Board   map[string]*BoardItem  `json:"board"`   // board items by ID
	Quiz    *QuizInfo              `json:"quiz"`    // the qualification quiz (nil until asked)
	Members map[string]*MemberInfo `json:"members"` // members by agent name
}

// busState is the subset of agentbus' state.json read by the live view.
type busState struct {
	Agents         map[string]*Presence      `json:"agents"`         // running agents by name
	Investigations map[string]*Investigation `json:"investigations"` // investigations by ID
	Teams          map[string]*TeamInfo      `json:"teams"`          // teams by ID
}

// Snapshot is the state of all tasks and the bus at one point in time.
type Snapshot struct {
	At     time.Time            // when the snapshot was taken
	Tasks  []*Task              // visible: active + finished but not acknowledged, sorted by priority
	All    map[string]*Task     // every task by ID
	Agents map[string]*Presence // running agents (bus presence) by name
	Invs   []*Investigation     // investigations sorted by ID
	Msgs   []Message            // tail of messages.jsonl
	Teams  map[string]*TeamInfo // teams by ID
}

// activeStatuses are task statuses of unfinished tasks (shown regardless of acknowledgement).
var activeStatuses = map[string]bool{"new": true, "starting": true, "running": true, "waiting": true, "investigating": true, "sleeping": true, "paused": true}

// Active reports whether the task has not finished yet (sleeping and waiting count as active).
func (t *Task) Active() bool { return activeStatuses[t.Status] }

// Label is the short name shown in the table (agent, inv#N or agent/HN).
func (t *Task) Label() string {
	switch t.Kind {
	case "investigation":
		return "inv#" + t.InvID
	case "helper":
		return t.Agent + "/H" + t.HelperID
	}
	return t.Agent
}

// BusName: the name under which the agent appears on the bus.
func (t *Task) BusName() string { return t.Agent }

// ---------------------------------------------------------------- reading state

// unquote reverses bash `printf %q` (values in meta.env).
func unquote(v string) string {
	if v == "''" {
		return ""
	}
	if strings.HasPrefix(v, "$'") && strings.HasSuffix(v, "'") && len(v) >= 3 {
		return ansiC(v[2 : len(v)-1])
	}
	var b strings.Builder
	esc := false
	for _, r := range v {
		if esc {
			b.WriteRune(r)
			esc = false
			continue
		}
		if r == '\\' {
			esc = true
			continue
		}
		b.WriteRune(r)
	}
	return b.String()
}

// ansiC decodes a bash $'...' (ANSI-C quoted) string.
func ansiC(s string) string {
	var out []byte
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c != '\\' || i+1 >= len(s) {
			out = append(out, c)
			continue
		}
		i++
		switch s[i] {
		case 'n':
			out = append(out, '\n')
		case 't':
			out = append(out, '\t')
		case 'r':
			out = append(out, '\r')
		case 'e', 'E':
			out = append(out, 0x1b)
		case 'x':
			j := i + 1
			for j < len(s) && j < i+3 && strings.ContainsRune("0123456789abcdefABCDEF", rune(s[j])) {
				j++
			}
			if n, err := strconv.ParseUint(s[i+1:j], 16, 8); err == nil {
				out = append(out, byte(n))
			}
			i = j - 1
		case '0', '1', '2', '3', '4', '5', '6', '7':
			j := i
			for j < len(s) && j < i+3 && s[j] >= '0' && s[j] <= '7' {
				j++
			}
			if n, err := strconv.ParseUint(s[i:j], 8, 8); err == nil {
				out = append(out, byte(n))
			}
			i = j - 1
		default:
			out = append(out, s[i])
		}
	}
	return string(out)
}

// readMeta parses a meta.env file (KEY=printf-%q-value lines); a missing file gives an empty map.
func readMeta(path string) map[string]string {
	m := map[string]string{}
	data, err := os.ReadFile(path) //nolint:gosec // path is a task's meta.env under AGENTS_HOME, which only the operator controls
	if errors.Is(err, os.ErrNotExist) {
		return m // a directory without meta.env is not a task
	}
	if err != nil {
		warnOnce("read:"+path, LogMetaUnreadable, storageErr(err, "cannot read %s", path), "file", path)
		return m
	}
	for _, line := range strings.Split(string(data), "\n") {
		k, v, ok := strings.Cut(line, "=")
		if ok && k != "" {
			m[k] = unquote(v)
		}
	}
	return m
}

// parseTime parses an RFC 3339 timestamp from meta.env. An empty value means "not set" (zero time); a
// malformed value is logged once and also reads as zero.
func parseTime(s string) time.Time {
	if s == "" {
		return time.Time{}
	}
	t, err := time.Parse(time.RFC3339, s)
	if err != nil {
		warnOnce("time:"+s, LogMetaMalformed, valueWrap(err, "malformed timestamp %q", s))
		return time.Time{}
	}
	return t
}

// parseCount parses a counter from meta.env (ROUND, RUN). An empty value is 0; a malformed one is logged once.
func parseCount(path, key, s string) int {
	if s == "" {
		return 0
	}
	n, err := strconv.Atoi(s)
	if err != nil {
		warnOnce(path+":"+key, LogMetaMalformed, valueWrap(err, "malformed %s %q", key, s), "file", path, "key", key)
		return 0
	}
	return n
}

// pidAlive reports whether the task's runner process is alive, and when the current run started.
func pidAlive(dir string) (bool, time.Time) {
	p := filepath.Join(dir, "runner.pid")
	st, err := os.Stat(p)
	if err != nil {
		return false, time.Time{}
	}
	data, err := os.ReadFile(p) //nolint:gosec // p is a file under AGENTS_HOME, which only the operator controls
	if err != nil {
		warnOnce(p, LogPIDFileUnreadable, storageErr(err, "cannot read %s", p), "file", p)
		return false, time.Time{}
	}
	pid, perr := strconv.Atoi(strings.TrimSpace(string(data)))
	if perr != nil || pid <= 0 {
		warnOnce("pid:"+p, LogPIDFileUnreadable, valueWrap(fmt.Errorf("%q", data), "runner.pid does not hold a PID"), "file", p)
		return false, time.Time{}
	}
	cmd, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/cmdline")
	return err == nil && bytes.Contains(cmd, []byte("_runner")), st.ModTime()
}

// tailLines returns the last n lines of a file (reads at most maxBytes from the end).
// A missing file has no lines; other read errors are logged once.
func tailLines(path string, n int, maxBytes int64) []string {
	f, err := os.Open(path) //nolint:gosec // path is a log file under AGENTS_HOME, which only the operator controls
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		warnOnce("open:"+path, LogFileReadFailed, storageErr(err, "cannot open %s", path), "file", path)
		return nil
	}
	defer closeQuietly(f, path) // read-only: a close error loses nothing, but it is still logged
	st, err := f.Stat()
	if err != nil {
		warnOnce("stat:"+path, LogFileReadFailed, storageErr(err, "cannot stat %s", path), "file", path)
		return nil
	}
	off := st.Size() - maxBytes
	if off < 0 {
		off = 0
	}
	if _, err := f.Seek(off, io.SeekStart); err != nil {
		warnOnce("seek:"+path, LogFileReadFailed, storageErr(err, "cannot seek in %s", path), "file", path)
		return nil
	}
	data, err := io.ReadAll(f)
	if err != nil {
		warnOnce("read:"+path, LogFileReadFailed, storageErr(err, "cannot read %s", path), "file", path)
		return nil
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if off > 0 && len(lines) > 0 {
		lines = lines[1:] // the first line may have been cut
	}
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	if len(lines) == 1 && lines[0] == "" {
		return nil
	}
	return lines
}

// readActivity returns the last n activity entries of a task.
func readActivity(dir string, n int) []Activity {
	var out []Activity
	path := filepath.Join(dir, "activity.jsonl")
	for _, l := range tailLines(path, n, 512*1024) {
		var a Activity
		if err := json.Unmarshal([]byte(l), &a); err != nil {
			warnOnce("activity:"+path+":"+l, LogCorruptActivityLine, valueWrap(err, "corrupt line in %s", path), "file", path)
			continue
		}
		out = append(out, a)
	}
	return out
}

// loadSnapshot reads all task directories and the bus state.
func loadSnapshot(home, busDir string) *Snapshot {
	s := &Snapshot{At: time.Now(), All: map[string]*Task{}, Agents: map[string]*Presence{}}
	tasksDir := filepath.Join(home, "tasks")
	entries, err := os.ReadDir(tasksDir)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		warnOnce("readdir:"+tasksDir, LogTasksListFailed, storageErr(err, "cannot list %s", tasksDir), "dir", tasksDir)
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		dir := filepath.Join(tasksDir, e.Name())
		m := readMeta(filepath.Join(dir, "meta.env"))
		if m["ID"] == "" {
			continue
		}
		t := &Task{
			ID: m["ID"], Agent: m["AGENT"], Kind: m["KIND"], Type: m["TYPE"], Model: m["MODEL"],
			Repo: m["REPO_NAME"], Branch: m["BRANCH"], Worktree: m["WORKTREE"], Status: m["STATUS"],
			InvID: m["INV_ID"], ParentTask: m["PARENT_TASK"], FromAgent: m["FROM_AGENT"], Team: m["TEAM"], HelperID: m["HELPER_ID"], Tier: m["TIER"],
			Created: parseTime(m["CREATED"]), Finished: parseTime(m["FINISHED"]), Dir: dir,
		}
		if t.Kind == "" {
			t.Kind = "worker"
		}
		t.Round = parseCount(filepath.Join(dir, "meta.env"), "ROUND", m["ROUND"])
		t.Run = parseCount(filepath.Join(dir, "meta.env"), "RUN", m["RUN"])
		t.Alive, t.RunStart = pidAlive(dir)
		_, err := os.Stat(filepath.Join(dir, "acked"))
		t.Acked = err == nil
		if a := readActivity(dir, 1); len(a) > 0 {
			t.Last = a[0]
		}
		s.All[t.ID] = t
		if t.Active() || !t.Acked {
			s.Tasks = append(s.Tasks, t)
		}
	}
	sort.SliceStable(s.Tasks, func(i, j int) bool {
		pi, pj := statusPrio(s.Tasks[i]), statusPrio(s.Tasks[j])
		if pi != pj {
			return pi < pj
		}
		return s.Tasks[i].ID < s.Tasks[j].ID
	})

	var st busState
	statePath := filepath.Join(busDir, "state.json")
	data, err := os.ReadFile(statePath) //nolint:gosec // statePath is a state file under AGENTS_HOME, which only the operator controls
	switch {
	case errors.Is(err, os.ErrNotExist): // no bus state yet
	case err != nil:
		warnOnce("read:"+statePath, LogBusStateUnreadable, storageErr(err, "cannot read %s", statePath), "file", statePath)
	default:
		if err := json.Unmarshal(data, &st); err != nil {
			warnOnce("decode:"+statePath, LogBusStateUnreadable, valueWrap(err, "cannot decode %s", statePath), "file", statePath)
		}
	}
	if st.Agents != nil {
		s.Agents = st.Agents
	}
	s.Teams = st.Teams
	if s.Teams == nil {
		s.Teams = map[string]*TeamInfo{}
	}
	for _, inv := range st.Investigations {
		s.Invs = append(s.Invs, inv)
	}
	sort.Slice(s.Invs, func(i, j int) bool { return s.Invs[i].ID < s.Invs[j].ID })
	for _, l := range tailLines(filepath.Join(busDir, "messages.jsonl"), 300, 1024*1024) {
		var m Message
		if json.Unmarshal([]byte(l), &m) == nil {
			s.Msgs = append(s.Msgs, m)
		}
	}
	return s
}

// statusPrio orders tasks: waiting for the human first, then running, investigating, sleeping, finished.
func statusPrio(t *Task) int {
	switch t.Status {
	case "waiting":
		return 0
	case "running", "starting", "new":
		return 1
	case "investigating":
		return 2
	case "sleeping", "paused":
		return 3
	default:
		return 4
	}
}

// QueueLine summarises the investigation queue.
// InvSlots: number of investigators (flag --inv-slots).
var InvSlots = 1

// QueueLine summarises the investigation queue in one line.
func (s *Snapshot) QueueLine(queueMax int) string {
	active, queued, wait := 0, 0, 0
	for _, inv := range s.Invs {
		switch inv.Status {
		case "starting", "running", "waiting_human":
			active++
		case "queued":
			queued++
		case "waitroom":
			wait++
		}
	}
	if free := InvSlots - active; free > 0 {
		queued -= free
		if queued < 0 {
			queued = 0
		}
	}
	return fmt.Sprintf("investigations: in progress %d/%d, waiting %d/%d, waiting room %d", active, InvSlots, queued, queueMax, wait)
}

// Waiting returns tasks waiting for the human's answers.
func (s *Snapshot) Waiting() []*Task {
	var out []*Task
	for _, t := range s.Tasks {
		if t.Status == "waiting" {
			out = append(out, t)
		}
	}
	return out
}

// ---------------------------------------------------------------- events

// Event is one line of the event stream (feed mode and the events pane).
type Event struct {
	TS   time.Time // when the event was detected
	Kind string    // status | bus | act | remind | info | err
	Text string    // the rendered event line
}

// Watcher polls snapshots and turns differences into events (status changes, new bus messages,
// new activity entries, periodic reminders).
type Watcher struct {
	home, busDir string           // AGENTS_HOME and the bus directory
	prev         *Snapshot        // previous snapshot (nil before the first poll)
	lastMsg      int64            // last message ID already reported
	offsets      map[string]int64 // read offset in each task's activity.jsonl
	lastRemind   time.Time        // when the last reminder was emitted
	remind       time.Duration    // reminder interval (0 = off)
}

// NewWatcher creates a watcher; the first Poll only records the baseline.
func NewWatcher(home, busDir string, remind time.Duration) *Watcher {
	return &Watcher{home: home, busDir: busDir, offsets: map[string]int64{}, remind: remind, lastRemind: time.Now()}
}

// finalSummary returns the first lines of a task's last agent message (final.md) joined with " | ".
func finalSummary(dir string) string {
	data, err := os.ReadFile(filepath.Join(dir, "final.md")) //nolint:gosec // final.md inside a task directory under AGENTS_HOME, which only the operator controls
	if err != nil {
		return ""
	}
	var parts []string
	for _, l := range strings.Split(string(data), "\n") {
		if l = strings.TrimSpace(l); l != "" {
			parts = append(parts, l)
		}
		if len(parts) == 4 {
			break
		}
	}
	return strings.Join(parts, " | ")
}

// newActivity returns new activity log entries. fromStart: the task appeared
// while the view is running — read the file from the beginning.
func (w *Watcher) newActivity(t *Task, fromStart bool) []Activity {
	path := filepath.Join(t.Dir, "activity.jsonl")
	if fromStart {
		if _, seen := w.offsets[t.ID]; !seen {
			w.offsets[t.ID] = 0
		}
	}
	st, err := os.Stat(path)
	if err != nil {
		return nil
	}
	off, seen := w.offsets[t.ID]
	if !seen || st.Size() < off {
		w.offsets[t.ID] = st.Size()
		return nil
	}
	f, err := os.Open(path) //nolint:gosec // path is a log file under AGENTS_HOME, which only the operator controls
	if err != nil {
		warnOnce("open:"+path, LogFileReadFailed, storageErr(err, "cannot open %s", path), "file", path)
		return nil
	}
	defer closeQuietly(f, path)
	if _, err := f.Seek(off, io.SeekStart); err != nil {
		warnOnce("seek:"+path, LogFileReadFailed, storageErr(err, "cannot seek in %s", path), "file", path)
		return nil
	}
	var out []Activity
	r := bufio.NewReader(f)
	for {
		line, err := r.ReadBytes('\n')
		if err != nil { // incomplete line — read it later
			break
		}
		off += int64(len(line))
		var a Activity
		if json.Unmarshal(line, &a) == nil {
			out = append(out, a)
		}
	}
	w.offsets[t.ID] = off
	return out
}

// Poll loads a new snapshot and returns the events since the previous one.
func (w *Watcher) Poll() (*Snapshot, []Event) {
	cur := loadSnapshot(w.home, w.busDir)
	now := cur.At
	var ev []Event
	if w.prev == nil {
		for _, m := range cur.Msgs {
			w.lastMsg = m.ID
		}
		for _, t := range cur.All {
			w.newActivity(t, false) // sets offsets to the end of the files
		}
		w.prev = cur
		return cur, nil
	}
	ids := make([]string, 0, len(cur.All))
	for id := range cur.All {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		t := cur.All[id]
		p := w.prev.All[id]
		switch {
		case p == nil:
			what := firstLine(filepath.Join(t.Dir, "task.md"))
			if t.Kind == "investigation" {
				what = "for " + t.FromAgent + ": " + what
			}
			ev = append(ev, Event{now, "info", fmt.Sprintf("new task %s [%s]: %s", t.Label(), t.Repo, what)})
		case p.Status != t.Status:
			text := fmt.Sprintf("%s [%s]: %s → %s", t.Label(), t.Repo, p.Status, t.Status)
			kind := "status"
			switch t.Status {
			case "done", "failed", "stopped":
				if sum := finalSummary(t.Dir); sum != "" {
					text += " — " + sum
				}
				if t.Status != "done" {
					kind = "err"
				}
			case "waiting":
				text += " — waiting for your answers (key a in the panel, or: agent answer " + t.ID + ")"
			}
			ev = append(ev, Event{now, kind, text})
		}
		if t.Active() || (p != nil && p.Active()) {
			for _, a := range w.newActivity(t, p == nil) {
				ev = append(ev, Event{now, "act", fmt.Sprintf("%s %s %s", t.Label(), actIcon(a.Kind), a.Text)})
			}
		}
	}
	for _, m := range cur.Msgs {
		if m.ID > w.lastMsg {
			ev = append(ev, Event{now, "bus", fmt.Sprintf("[bus] %s → %s: %s", m.From, m.To, m.Text)})
			w.lastMsg = m.ID
		}
	}
	if w.remind > 0 && now.Sub(w.lastRemind) >= w.remind {
		w.lastRemind = now
		if r := remindText(cur); r != "" {
			ev = append(ev, Event{now, "remind", r})
		}
	}
	w.prev = cur
	return cur, ev
}

// remindText builds the periodic "still working / waiting for you / sleeping" reminder.
func remindText(s *Snapshot) string {
	var work, wait, sleep []string
	for _, t := range s.Tasks {
		switch t.Status {
		case "running", "starting", "investigating":
			d := ""
			if !t.RunStart.IsZero() {
				d = ", " + human(time.Since(t.RunStart))
			}
			stage := ""
			if p := s.Agents[t.BusName()]; p != nil && p.Status != "" && p.Task == t.ID {
				stage = ", „" + trunc(p.Status, 50) + "”"
			}
			work = append(work, fmt.Sprintf("%s (%s%s%s)", t.Label(), t.Status, d, stage))
		case "waiting":
			wait = append(wait, t.Label())
		case "sleeping":
			sleep = append(sleep, t.Label())
		case "paused":
			sleep = append(sleep, t.Label()+" (paused)")
		}
	}
	if len(work) == 0 && len(wait) == 0 {
		return ""
	}
	out := ""
	if len(work) > 0 {
		out = "still working: " + strings.Join(work, ", ")
	}
	if len(wait) > 0 {
		if out != "" {
			out += " | "
		}
		out += "waiting for you: " + strings.Join(wait, ", ")
	}
	if len(sleep) > 0 {
		out += " | sleeping: " + strings.Join(sleep, ", ")
	}
	return out
}

// firstLine: the first non-empty line that is not a markdown heading (or the heading if nothing else).
func firstLine(path string) string {
	data, err := os.ReadFile(path) //nolint:gosec // path is under AGENTS_HOME, which only the operator controls
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		warnOnce("read:"+path, LogFileReadFailed, storageErr(err, "cannot read %s", path), "file", path)
	}
	header := ""
	for _, l := range strings.Split(string(data), "\n") {
		l = strings.TrimSpace(l)
		switch {
		case l == "":
		case strings.HasPrefix(l, "#"):
			if header == "" {
				header = strings.TrimSpace(strings.TrimLeft(l, "#"))
			}
		default:
			return trunc(l, 100)
		}
	}
	return trunc(header, 100)
}

// trunc collapses whitespace and shortens s to n runes.
func trunc(s string, n int) string {
	s = strings.Join(strings.Fields(s), " ")
	r := []rune(s)
	if len(r) <= n {
		return s
	}
	return string(r[:n]) + "…"
}

// human formats a duration as 42s, 7m or 1h05m.
func human(d time.Duration) string {
	switch {
	case d < time.Minute:
		return fmt.Sprintf("%ds", int(d.Seconds()))
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	default:
		return fmt.Sprintf("%dh%02dm", int(d.Hours()), int(d.Minutes())%60)
	}
}

// shortTS turns "2026-10-07 15:04:05" into "15:04:05" (older entries already contain only the time).
func shortTS(ts string) string {
	if len(ts) > 8 {
		return ts[len(ts)-8:]
	}
	return ts
}

// actIcon maps an activity kind to its icon.
func actIcon(k string) string {
	switch k {
	case "text":
		return "»"
	case "cmd":
		return "$"
	case "edit":
		return "✎"
	case "mcp":
		return "⇄"
	case "err":
		return "✗"
	case "info":
		return "·"
	default:
		return "⚙"
	}
}

// ---------------------------------------------------------------- repository notebook

// safeName maps a repository name to its notebook directory name (same rule as agentbus).
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

// noteSection is a repository notebook section (see agentbus NoteSection).
type noteSection struct {
	Text    string `json:"text"`    // section content
	Author  string `json:"author"`  // agent that wrote it
	Updated string `json:"updated"` // when it was written
}

// noteEntry is a repository notebook log entry (see agentbus NoteEntry).
type noteEntry struct {
	TS     string `json:"ts"`     // when it was added
	Author string `json:"author"` // agent that added it
	Tag    string `json:"tag"`    // optional topic
	Text   string `json:"text"`   // the finding
}

// Notebook returns the sections (full text) and the latest log entries of a repository notebook.
func Notebook(busDir, repo string, last int) (map[string]noteSection, []noteEntry) {
	dir := filepath.Join(busDir, "notebooks", safeName(repo))
	secs := map[string]noteSection{}
	secPath := filepath.Join(dir, "sections.json")
	data, err := os.ReadFile(secPath) //nolint:gosec // secPath is a file under AGENTS_HOME, which only the operator controls
	switch {
	case errors.Is(err, os.ErrNotExist): // no sections yet
	case err != nil:
		warnOnce("read:"+secPath, LogNotebookUnreadable, storageErr(err, "cannot read %s", secPath), "file", secPath)
	default:
		if err := json.Unmarshal(data, &secs); err != nil {
			warnOnce("decode:"+secPath, LogNotebookUnreadable, valueWrap(err, "cannot decode %s", secPath), "file", secPath)
		}
	}
	var notes []noteEntry
	logPath := filepath.Join(dir, "log.jsonl")
	for _, l := range tailLines(logPath, last, 2*1024*1024) {
		var e noteEntry
		if err := json.Unmarshal([]byte(l), &e); err != nil {
			warnOnce("note:"+logPath+":"+l, LogCorruptNoteLine, valueWrap(err, "corrupt line in %s", logPath), "file", logPath)
		} else {
			notes = append(notes, e)
		}
	}
	return secs, notes
}

// closeQuietly closes a file opened for reading; a close error cannot lose data, but it is logged once.
func closeQuietly(f *os.File, path string) {
	if err := f.Close(); err != nil {
		warnOnce("close:"+path, LogFileReadFailed, storageErr(err, "cannot close %s", path), "file", path)
	}
}
