// agentbus — a minimal MCP server (stdio) for agent communication, coordination and resource reservations.
//
// Go standard library only. Every agent (Claude Code, Codex CLI, ...) starts
// its own `agentbus serve` instance; all instances share state in the --dir directory:
//
//	state.json      — active agents, reservations, read cursors, investigations, teams, worker requests
//	messages.jsonl  — messages (append-only log)
//	bus.lock        — flock guarding both files
//
// An agent is "active" while the process of its agentbus instance is alive (checked via
// /proc/<pid>/cmdline). Reservations of an agent that disappeared are released automatically.
//
// Human-facing subcommands: send, log, status, release.
package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
	"time"
)

const (
	// version is reported to MCP clients in the initialize response (serverInfo.version).
	version = "0.6.0"
	// defaultProto is the MCP protocol version we answer with when the client does not send one.
	defaultProto = "2025-06-18"
	// maxWaitSeconds caps every blocking tool call (reserve with wait_seconds, wait_for_message): one call
	// never blocks an agent for more than 10 minutes, which also keeps it well under the hang watchdog.
	maxWaitSeconds = 600
	// pollInterval is how often blocking calls re-check the shared state while they wait.
	pollInterval = 3 * time.Second
)

// ---------------------------------------------------------------- state

// Message is one entry in the append-only message log (messages.jsonl).
type Message struct {
	ID   int64           `json:"id"`             // monotonically increasing; read cursors compare against it
	TS   Timestamp       `json:"ts"`             // local time, "2006-01-02 15:04:05"
	From string          `json:"from"`           // sender: agent name, "agentbus" for system notices, or the human's user name
	To   string          `json:"to"`             // recipient: agent name, "all", or "team:<team id>"
	Kind MessageKind     `json:"kind,omitempty"` // event type (see events.go); "" in messages of older versions = chat
	Data json.RawMessage `json:"data,omitempty"` // the event's fields as JSON (for log pipelines and tools)
	Text string          `json:"text"`           // the rendered text agents and humans read
}

// Presence describes a currently running agent. It exists only while the process of the
// agent's `agentbus serve` instance is alive (see alive and Bus.gc).
type Presence struct {
	Agent    string    `json:"agent"`     // agent name from agents.conf (e.g. "claude-1")
	Task     string    `json:"task"`      // task ID the agent is working on
	Repo     string    `json:"repo"`      // repository name (used e.g. as the notebook key)
	Branch   string    `json:"branch"`    // the agent's git branch
	Worktree string    `json:"worktree"`  // absolute path of the agent's git worktree
	Status   string    `json:"status"`    // free-form stage text set with set_status
	Started  Timestamp `json:"started"`   // when this presence entry was created
	LastSeen Timestamp `json:"last_seen"` // updated on every tool call
	PID      int       `json:"pid"`       // PID of the agent's agentbus process (liveness check)
}

// Reservation marks a shared resource (debug probe, serial port, test host, ...) as taken.
// It is released explicitly, or automatically when the holding agent disappears.
type Reservation struct {
	Resource string    `json:"resource"` // free-form resource name agreed in the environment notes
	Agent    string    `json:"agent"`    // holder
	Task     string    `json:"task"`     // holder's task
	Note     string    `json:"note"`     // why it was reserved
	Since    Timestamp `json:"since"`    // when the reservation was made
}

// Investigation is a request handled by an investigator (an advanced, expensive model):
// either an agent asking for help with a hard problem (request_investigation), or a PR
// review queued by the `agent` script for teams started with --verify.
//
// Status lifecycle: queued | waitroom → starting → running ⇄ waiting_human → done | failed;
// "rejected" when the queue was full and the agent chose not to wait.
type Investigation struct {
	Investigator  string              `json:"investigator,omitempty"`  // assigned investigator base name; absent in legacy local records
	Instance      string              `json:"instance,omitempty"`      // instance owning the investigator task; remote admission requires this binding
	ID            int64               `json:"id"`                      // sequential number shown as "#ID" to agents and humans
	FromAgent     string              `json:"from_agent"`              // requesting agent
	FromTask      string              `json:"from_task"`               // requesting task (the result is delivered to it)
	Repo          string              `json:"repo"`                    // repository name of the requesting task
	Branch        string              `json:"branch"`                  // requester's branch; the investigator starts from its tip
	Worktree      string              `json:"worktree"`                // requester's worktree (read-only for the investigator)
	Question      string              `json:"question"`                // what the requester wants investigated (one specific question)
	Context       string              `json:"context"`                 // what the requester already knows: checks done, hypotheses, relevant files
	Status        InvestigationStatus `json:"status"`                  // lifecycle state, see the type comment
	Created       Timestamp           `json:"created"`                 // when the request was made
	Started       Timestamp           `json:"started,omitempty"`       // when an investigator picked it up
	Finished      Timestamp           `json:"finished,omitempty"`      // when it ended (done, failed or cancelled)
	InvTask       string              `json:"inv_task,omitempty"`      // task ID of the investigator run
	File          string              `json:"file"`                    // markdown record: question → result → decision
	Summary       string              `json:"summary,omitempty"`       // one-line result summary
	Decision      string              `json:"decision,omitempty"`      // requester's decision after the result (record_decision)
	Justification string              `json:"justification,omitempty"` // why the requester decided so, including how it verified the result
	DecidedAt     Timestamp           `json:"decided_at,omitempty"`    // when the decision was recorded
	Kind          InvestigationKind   `json:"kind,omitempty"`          // "" = investigation | "review" = PR review (agent run --team --verify)
	PRURL         string              `json:"pr_url,omitempty"`        // review only
	PRNumber      int                 `json:"pr_number,omitempty"`     // review only
}

// State is the whole shared bus state, persisted in state.json and always accessed under
// the bus lock (Bus.with). Map keys are IDs/names as strings.
type State struct {
	NextID         int64                     `json:"next_id"`        // next message ID
	Agents         map[string]*Presence      `json:"agents"`         // running agents by name
	Reservations   map[string]*Reservation   `json:"reservations"`   // taken resources by resource name
	Cursors        map[string]int64          `json:"cursors"`        // last read message ID per agent
	NextInvID      int64                     `json:"next_inv_id"`    // last investigation ID issued
	Investigations map[string]*Investigation `json:"investigations"` // by ID
	Teams          map[string]*Team          `json:"teams"`          // by team ID
	NextHelperID   int64                     `json:"next_helper_id"` // last worker request ID issued
	Helpers        map[string]*Helper        `json:"helpers"`        // worker requests (delegate) by ID
}

// Bus is a handle to the shared state directory (state.json, messages.jsonl, bus.lock,
// notebooks/, checkpoints/).
type Bus struct {
	dir string // absolute path of the state directory (default ~/agents/bus)
}

// path returns the absolute path of a file in the state directory.
func (b *Bus) path(n string) string { return filepath.Join(b.dir, n) }

// load reads state.json (missing file = empty state) and initialises nil maps. An unknown enum value in the
// file is a decoding error (see types.go).
func (b *Bus) load() (*State, *StorageError) {
	st := &State{}
	data, err := os.ReadFile(b.path("state.json"))
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, storageErr(err, "cannot read %s", b.path("state.json"))
	}
	if len(data) > 0 {
		if err := json.Unmarshal(data, st); err != nil {
			var verr *ValueError
			if errors.As(err, &verr) {
				return nil, storageErr(err, "cannot decode %s: it contains a value this version does not know "+
					"(written by a newer version or edited by hand; see README, Troubleshooting)", b.path("state.json"))
			}
			return nil, storageErr(err, "cannot decode %s", b.path("state.json"))
		}
	}
	if st.Agents == nil {
		st.Agents = map[string]*Presence{}
	}
	if st.Reservations == nil {
		st.Reservations = map[string]*Reservation{}
	}
	if st.Cursors == nil {
		st.Cursors = map[string]int64{}
	}
	if st.Investigations == nil {
		st.Investigations = map[string]*Investigation{}
	}
	if st.Teams == nil {
		st.Teams = map[string]*Team{}
	}
	if st.Helpers == nil {
		st.Helpers = map[string]*Helper{}
	}
	if st.NextID == 0 {
		st.NextID = 1
	}
	return st, nil
}

// save writes state.json atomically (temp file + rename).
func (b *Bus) save(st *State) *StorageError {
	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return storageErr(err, "cannot encode the bus state")
	}
	tmp := b.path("state.json.tmp")
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return storageErr(err, "cannot write %s", tmp)
	}
	if err := os.Rename(tmp, b.path("state.json")); err != nil {
		return storageErr(err, "cannot replace %s", b.path("state.json"))
	}
	return nil
}

// locked runs fn on the shared state under an exclusive lock (after removing stale entries) and saves the
// state afterwards. Only storage can fail here; failures of the operation itself are fn's business.
func (b *Bus) locked(fn func(*State)) *StorageError {
	if err := os.MkdirAll(b.dir, 0o700); err != nil {
		return storageErr(err, "cannot create the state directory %s", b.dir)
	}
	f, err := os.OpenFile(b.path("bus.lock"), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return storageErr(err, "cannot open the lock file")
	}
	defer func() {
		if err := f.Close(); err != nil {
			warnSideEffect("close lock file", storageErr(err, "cannot close the lock file"))
		}
	}()
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX); err != nil {
		return storageErr(err, "cannot lock the bus state")
	}
	defer func() {
		if err := syscall.Flock(int(f.Fd()), syscall.LOCK_UN); err != nil {
			warnSideEffect("unlock", storageErr(err, "cannot unlock the bus state"))
		}
	}()
	st, serr := b.load()
	if serr != nil {
		return serr
	}
	b.gc(st)
	fn(st)
	return b.save(st)
}

// transact runs one operation on the shared state. A storage failure (lock, load, save) becomes the
// operation's storage cause; otherwise the operation's own error (or nil) is returned.
func transact[C opCause](b *Bus, storage C, fn func(*State) *OpError[C]) *OpError[C] {
	var opErr *OpError[C]
	if serr := b.locked(func(st *State) { opErr = fn(st) }); serr != nil {
		return fromStorage(storage, serr)
	}
	return opErr
}

// alive reports whether pid belongs to a running agentbus process (guards against PID reuse).
func alive(pid int) bool {
	if pid <= 0 {
		return false
	}
	cmd, err := os.ReadFile(fmt.Sprintf("/proc/%d/cmdline", pid))
	return err == nil && bytes.Contains(cmd, []byte("agentbus"))
}

// gc removes presences of dead agentbus processes and releases their reservations. Announcing an
// automatic release is a side effect of whatever operation triggered the cleanup, so a failure is logged.
func (b *Bus) gc(st *State) {
	for name, p := range st.Agents {
		if !alive(p.PID) {
			delete(st.Agents, name)
			logInfo(LogAgentGone, "agent", name, "agent_pid", p.PID)
		}
	}
	for res, r := range st.Reservations {
		if _, ok := st.Agents[r.Agent]; !ok {
			delete(st.Reservations, res)
			ev := EventAutoReleased{Resource: res, Holder: r.Agent, Reason: ReleaseAgentGone}
			if _, err := b.post(st, "agentbus", "all", ev); err != nil {
				warnSideEffect("announce auto-release", err, "resource", res, "holder", r.Agent)
			}
		}
	}
}

// teamRecipient returns the message recipient that addresses every member of a team.
func teamRecipient(teamID string) string { return "team:" + teamID }

// post appends an event to messages.jsonl (caller holds the lock) and wakes sleeping recipients.
func (b *Bus) post(st *State, from, to string, ev busEvent) (Message, *StorageError) {
	data, err := json.Marshal(ev)
	if err != nil {
		return Message{}, storageErr(err, "cannot encode a %s event", ev.Kind())
	}
	m := Message{ID: st.NextID, TS: Now(), From: from, To: to, Kind: ev.Kind(), Data: data, Text: ev.Text()}
	line, err := json.Marshal(m)
	if err != nil {
		return Message{}, storageErr(err, "cannot encode message #%d", m.ID)
	}
	if serr := appendFile(b.path("messages.jsonl"), string(line)+"\n"); serr != nil {
		return Message{}, serr
	}
	st.NextID++
	logInfo(LogMessagePosted, "message_id", m.ID, "kind", m.Kind, "from", from, "to", to)
	b.wakeFor(st, m)
	return m, nil
}

// messages reads the whole message log. A corrupt line is skipped (and reported once) so that it does not
// hide all other messages.
func (b *Bus) messages() ([]Message, *StorageError) {
	f, err := os.Open(b.path("messages.jsonl"))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read the message log")
	}
	defer func() {
		if err := f.Close(); err != nil {
			warnSideEffect("close message log", storageErr(err, "cannot close the message log"))
		}
	}()
	var out []Message
	lineNo := 0
	r := bufio.NewReader(f)
	for {
		line, rerr := r.ReadBytes('\n')
		lineNo++
		if len(bytes.TrimSpace(line)) > 0 {
			var m Message
			if err := json.Unmarshal(line, &m); err != nil {
				warnOnce(fmt.Sprintf("messages.jsonl:%d", lineNo), LogCorruptMessageLine,
					storageErr(err, "cannot decode line %d of the message log", lineNo))
			} else {
				out = append(out, m)
			}
		}
		if errors.Is(rerr, io.EOF) {
			break
		}
		if rerr != nil {
			return nil, storageErr(rerr, "cannot read the message log")
		}
	}
	return out, nil
}

// fmtMsg renders a message as a single line.
func fmtMsg(m Message) string {
	return fmt.Sprintf("[#%d %s] %s → %s: %s", m.ID, m.TS, m.From, m.To, m.Text)
}

// ---------------------------------------------------------------- MCP server

// Server is one `agentbus serve` instance — the MCP server of a single agent run.
type Server struct {
	bus       *Bus                // shared state
	me        string              // this agent's name
	info      Presence            // presence template (task, repo, branch, worktree)
	peers     []string            // configured agent names (shown as inactive when not running)
	out       *json.Encoder       // JSON-RPC output (stdout)
	role      Role                // regular agent or investigator
	queueMax  int                 // investigation queue limit (waiting requests)
	tasksDir  string              // tasks directory (investigation records live under it)
	team      string              // team ID when running in team mode, else ""
	tier      Tier                // normal or worker (a cheap model: cannot coordinate or delegate)
	agentCmd  string              // path to the `agent` script (to spawn helper dispatch)
	home      string              // AGENTS_HOME (passed to spawned processes)
	hubClient *HubClient          // nonnil forwards tools without touching a local bus
	hubCaller HubCaller           // registered identity used for forwarded requests
	instance  string              // instance this server runs on (capability checks for board_claim)
	directory CapabilityDirectory // who can run work; nil provides no capability (fails closed for requires)
}

// touch (re)registers this agent's presence and updates LastSeen.
func (s *Server) touch(st *State) {
	p := st.Agents[s.me]
	if p == nil || p.PID != os.Getpid() {
		cp := s.info
		cp.PID = os.Getpid()
		cp.Started = Now()
		if p != nil {
			cp.Status = p.Status
		}
		p = &cp
		st.Agents[s.me] = p
	}
	p.LastSeen = Now()
}

// register creates this agent's presence at startup.
func (s *Server) register() *StorageError {
	return s.bus.locked(func(st *State) { s.touch(st) })
}

// unregister removes this agent's presence on exit and releases its reservations — unless a newer instance
// of the same agent has taken over the presence. A failure is logged: the presence then stays until the next
// gc notices the dead process.
func (s *Server) unregister() {
	serr := s.bus.locked(func(st *State) {
		p := st.Agents[s.me]
		if p == nil || p.PID != os.Getpid() {
			return // a newer instance of this agent took over the presence entry
		}
		delete(st.Agents, s.me)
		for res, r := range st.Reservations {
			if r.Agent != s.me {
				continue
			}
			delete(st.Reservations, res)
			ev := EventAutoReleased{Resource: res, Holder: s.me, Reason: ReleaseAgentFinished}
			if _, serr := s.bus.post(st, "agentbus", "all", ev); serr != nil {
				warnSideEffect("announce auto-release", serr, "resource", res)
			}
		}
	})
	if serr != nil {
		warnSideEffect("unregister", serr)
	}
}

// unreadCount counts unread messages addressed to this agent (direct, team or all).
func (s *Server) unreadCount() (int, *StorageError) {
	n := 0
	var readErr *StorageError
	serr := s.bus.locked(func(st *State) {
		msgs, err := s.bus.messages()
		if err != nil {
			readErr = err
			return
		}
		cur := st.Cursors[s.me]
		for _, m := range msgs {
			if m.ID > cur && isForAgent(st, m, s.me, true) {
				n++
			}
		}
	})
	if serr != nil {
		return 0, serr
	}
	return n, readErr
}

// listAgents (MCP list_agents) shows running and configured-but-inactive agents.
func (s *Server) listAgents() (string, *ListAgentsError) {
	var sb strings.Builder
	err := transact(s.bus, ListAgentsStorage, func(st *State) *ListAgentsError {
		s.touch(st)
		names := make([]string, 0, len(st.Agents))
		for n := range st.Agents {
			names = append(names, n)
		}
		sort.Strings(names)
		fmt.Fprintf(&sb, "You: %s (task %s, repo %s, branch %s)\n\nActive agents:\n",
			s.me, s.info.Task, s.info.Repo, s.info.Branch)
		for _, n := range names {
			p := st.Agents[n]
			self := ""
			if n == s.me {
				self = " [you]"
			}
			status := p.Status
			if status == "" {
				status = "(no status)"
			}
			fmt.Fprintf(&sb, "- %s%s — task %s, repo %s, branch %s, worktree %s, since %s, status: %s\n",
				n, self, p.Task, p.Repo, p.Branch, p.Worktree, p.Started, status)
		}
		var idle []string
		for _, n := range s.peers {
			if _, ok := st.Agents[n]; !ok && n != "" {
				idle = append(idle, n)
			}
		}
		if len(idle) > 0 {
			fmt.Fprintf(&sb, "\nConfigured, currently inactive: %s (messages to them will wait)\n",
				strings.Join(idle, ", "))
		}
		return nil
	})
	return sb.String(), err
}

// send (MCP send_message) posts a message to an agent, "team" or "all".
func (s *Server) send(to, text string) (string, *SendMessageError) {
	to, text = strings.TrimSpace(to), strings.TrimSpace(text)
	if to == "" || text == "" {
		return "", opErr(SendMessageMissingArgument, "%s", missing("to", "text"))
	}
	if to == s.me {
		return "", opErr(SendMessageToSelf, "do not send messages to yourself")
	}
	if to == "team" {
		if s.team == "" {
			return "", opErr(SendMessageNotInTeam, "you are not in a team — use an agent name or 'all'")
		}
		to = teamRecipient(s.team)
	}
	var out string
	err := transact(s.bus, SendMessageStorage, func(st *State) *SendMessageError {
		s.touch(st)
		m, serr := s.bus.post(st, s.me, to, EventChat{Body: text})
		if serr != nil {
			return fromStorage(SendMessageStorage, serr)
		}
		out = "Sent: " + fmtMsg(m)
		return nil
	})
	return out, err
}

// read (MCP read_messages) returns unread messages and advances the cursor; with history it
// returns the last N messages of everyone without touching the cursor.
func (s *Server) read(history int) (string, *ReadMessagesError) {
	var sb strings.Builder
	err := transact(s.bus, ReadMessagesStorage, func(st *State) *ReadMessagesError {
		s.touch(st)
		msgs, serr := s.bus.messages()
		if serr != nil {
			return fromStorage(ReadMessagesStorage, serr)
		}
		if history > 0 {
			if history > 200 {
				history = 200
			}
			start := len(msgs) - history
			if start < 0 {
				start = 0
			}
			fmt.Fprintf(&sb, "Last %d messages (all directions):\n", len(msgs)-start)
			for _, m := range msgs[start:] {
				sb.WriteString(fmtMsg(m) + "\n")
			}
			return nil
		}
		cur := st.Cursors[s.me]
		n := 0
		for _, m := range msgs {
			if m.ID > cur && isForAgent(st, m, s.me, true) {
				sb.WriteString(fmtMsg(m) + "\n")
				n++
			}
		}
		if n == 0 {
			sb.WriteString("No new messages.\n")
		}
		st.Cursors[s.me] = st.NextID - 1
		return nil
	})
	return sb.String(), err
}

// setStatus (MCP set_status) sets the free-form stage text shown to others.
func (s *Server) setStatus(status string) (string, *SetStatusError) {
	status = strings.TrimSpace(status)
	err := transact(s.bus, SetStatusStorage, func(st *State) *SetStatusError {
		s.touch(st)
		st.Agents[s.me].Status = status
		return nil
	})
	if err != nil {
		return "", err
	}
	return "Status set.", nil
}

// reserve (MCP reserve) takes a shared resource; with wait it polls until the resource is free. "TAKEN" is a
// result, not an error: the agent decides what to do.
func (s *Server) reserve(res, note string, wait int) (string, *ReserveError) {
	res = strings.TrimSpace(res)
	if res == "" {
		return "", opErr(ReserveMissingResource, "%s", missing("resource"))
	}
	if wait < 0 {
		wait = 0
	}
	if wait > maxWaitSeconds {
		wait = maxWaitSeconds
	}
	deadline := time.Now().Add(time.Duration(wait) * time.Second)
	for {
		var msg string
		granted := false
		err := transact(s.bus, ReserveStorage, func(st *State) *ReserveError {
			s.touch(st)
			r, held := st.Reservations[res]
			switch {
			case !held:
				st.Reservations[res] = &Reservation{Resource: res, Agent: s.me, Task: s.info.Task, Note: note, Since: Now()}
				if _, serr := s.bus.post(st, s.me, "all", EventReserved{Resource: res, Note: note}); serr != nil {
					return fromStorage(ReserveStorage, serr)
				}
				granted, msg = true, "OK: reserved "+res+". Release it right after you are done."
			case r.Agent == s.me:
				r.Note = note
				granted, msg = true, "OK: "+res+" is already yours."
			default:
				msg = fmt.Sprintf("TAKEN: %s holds %s since %s (task %s)", r.Agent, res, r.Since, r.Task)
				if r.Note != "" {
					msg += ", purpose: " + r.Note
				}
				msg += ". You can message that agent or retry with wait_seconds."
			}
			return nil
		})
		if err != nil {
			return "", err
		}
		if granted || !time.Now().Before(deadline) {
			return msg, nil
		}
		time.Sleep(pollInterval)
	}
}

// release (MCP release) frees a resource held by the caller.
func (s *Server) release(res string) (string, *ReleaseError) {
	res = strings.TrimSpace(res)
	var msg string
	err := transact(s.bus, ReleaseStorage, func(st *State) *ReleaseError {
		s.touch(st)
		r, held := st.Reservations[res]
		switch {
		case !held:
			msg = res + " was not reserved."
		case r.Agent != s.me:
			return opErr(ReleaseNotHolder, "%s belongs to %s — you cannot release it", res, r.Agent)
		default:
			delete(st.Reservations, res)
			if _, serr := s.bus.post(st, s.me, "all", EventReleased{Resource: res}); serr != nil {
				return fromStorage(ReleaseStorage, serr)
			}
			msg = "OK: released " + res + "."
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	return msg, nil
}

// fmtReservations renders the reservation table.
func fmtReservations(st *State) string {
	if len(st.Reservations) == 0 {
		return "No reservations.\n"
	}
	keys := make([]string, 0, len(st.Reservations))
	for k := range st.Reservations {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	var sb strings.Builder
	for _, k := range keys {
		r := st.Reservations[k]
		fmt.Fprintf(&sb, "- %s: %s (task %s) since %s", k, r.Agent, r.Task, r.Since)
		if r.Note != "" {
			sb.WriteString(", purpose: " + r.Note)
		}
		sb.WriteString("\n")
	}
	return sb.String()
}

// listReservations (MCP list_reservations).
func (s *Server) listReservations() (string, *ListReservationsError) {
	var out string
	err := transact(s.bus, ListReservationsStorage, func(st *State) *ListReservationsError {
		s.touch(st)
		out = fmtReservations(st)
		return nil
	})
	return out, err
}

// toolArgs gives typed access to the arguments of one tools/call request. The arguments are validated
// against the tool's JSON schema first (validateArgs), so the accessors cannot meet a wrong type.
type toolArgs map[string]any

// str returns a string argument ("" when absent).
func (a toolArgs) str(k string) string {
	v, _ := a[k].(string) // validated: absent or a string
	return v
}

// num returns a numeric argument (0 when absent). Numbers may arrive as JSON numbers or numeric strings.
func (a toolArgs) num(k string) int {
	switch v := a[k].(type) {
	case float64:
		return int(v)
	case string:
		n, _ := strconv.Atoi(v) // validated: a numeric string
		return n
	}
	return 0
}

// list returns a string-array argument (nil when absent). validateArgs has checked every item.
func (a toolArgs) list(k string) []string {
	items, _ := a[k].([]any) // validated: absent or an array of strings
	out := make([]string, 0, len(items))
	for _, it := range items {
		if sv, ok := it.(string); ok {
			out = append(out, sv)
		}
	}
	return out
}

// flag returns a boolean argument (false when absent). Booleans may arrive as JSON booleans or "true"/"false".
func (a toolArgs) flag(k string) bool {
	switch v := a[k].(type) {
	case bool:
		return v
	case string:
		return v == "true"
	}
	return false
}

// toolSchemas maps every tool name to its declared argument types (from the MCP tool definitions).
func toolSchemas() map[string]map[string]string {
	defs := append(append(append(toolDefs(), invToolDefs(RoleAgent)...), teamToolDefs(true)...), collabToolDefs(true, TierNormal)...)
	defs = append(defs, consultToolDefs(true)...)
	defs = append(defs, capabilityToolDefs()...)
	out := map[string]map[string]string{}
	for _, d := range defs {
		props := map[string]string{}
		if schema, ok := d["inputSchema"].(map[string]any); ok {
			if ps, ok := schema["properties"].(map[string]any); ok {
				for name, p := range ps {
					if pm, ok := p.(map[string]any); ok {
						if t, ok := pm["type"].(string); ok {
							props[name] = t
						}
					}
				}
			}
		}
		out[d["name"].(string)] = props
	}
	return out
}

// validateArgs checks the arguments of tool name against its declared types.
func validateArgs(name string, a map[string]any) *ToolCallError {
	props, known := toolSchemas()[name]
	if !known {
		return opErr(ToolCallUnknownTool, "unknown tool %q", name)
	}
	for k, v := range a {
		want, declared := props[k]
		if !declared {
			continue // tolerate extra arguments some clients add
		}
		switch want {
		case "string":
			if _, ok := v.(string); !ok {
				return opErr(ToolCallInvalidArgument, "%s: argument %q must be a string, got %T", name, k, v)
			}
		case "number", "integer":
			switch n := v.(type) {
			case float64:
				if n != float64(int(n)) {
					return opErr(ToolCallInvalidArgument, "%s: argument %q must be a whole number, got %v", name, k, n)
				}
			case string:
				if _, err := strconv.Atoi(n); err != nil {
					return opErr(ToolCallInvalidArgument, "%s: argument %q must be a number, got %q", name, k, n)
				}
			default:
				return opErr(ToolCallInvalidArgument, "%s: argument %q must be a number, got %T", name, k, v)
			}
		case "array":
			items, ok := v.([]any)
			if !ok {
				return opErr(ToolCallInvalidArgument, "%s: argument %q must be an array, got %T", name, k, v)
			}
			for _, it := range items {
				if _, isStr := it.(string); !isStr {
					return opErr(ToolCallInvalidArgument, "%s: every item of %q must be a string, got %T", name, k, it)
				}
			}
		case "boolean":
			switch b := v.(type) {
			case bool:
			case string:
				if b != "true" && b != "false" {
					return opErr(ToolCallInvalidArgument, "%s: argument %q must be true or false, got %q", name, k, b)
				}
			default:
				return opErr(ToolCallInvalidArgument, "%s: argument %q must be a boolean, got %T", name, k, v)
			}
		}
	}
	return nil
}

// call dispatches an MCP tools/call request to its handler and logs the outcome. The error is the tool's own
// typed error (e.g. *QuizStartError) or a *ToolCallError for an unknown tool or a mistyped argument.
func (s *Server) call(name string, raw map[string]any) (string, error) {
	start := time.Now()
	out, err := s.callTool(name, raw)
	attrs := []any{"tool", name, "duration_ms", time.Since(start).Milliseconds()}
	if err != nil {
		class, cause := classOf(err)
		logWarn(LogToolCallFailed, append(attrs, "error", err.Error(), "class", class, "cause", cause)...)
	} else {
		logInfo(LogToolCall, attrs...)
	}
	return out, err
}

// callTool runs the handler of one MCP tool.
func (s *Server) callTool(name string, raw map[string]any) (string, error) {
	if err := validateArgs(name, raw); err != nil {
		return toolResult("", err)
	}
	a := toolArgs(raw)
	switch name {
	case "list_agents":
		return toolResult(s.listAgents())
	case "send_message":
		return toolResult(s.send(a.str("to"), a.str("text")))
	case "read_messages":
		return toolResult(s.read(a.num("history")))
	case "set_status":
		return toolResult(s.setStatus(a.str("status")))
	case "reserve":
		return toolResult(s.reserve(a.str("resource"), a.str("note"), a.num("wait_seconds")))
	case "release":
		return toolResult(s.release(a.str("resource")))
	case "list_reservations":
		return toolResult(s.listReservations())
	case "request_investigation":
		return toolResult(s.requestInvestigation(a.str("question"), a.str("context"), a.flag("wait_if_full")))
	case "record_decision":
		return toolResult(s.recordDecision(a.num("investigation_id"), a.str("decision"), a.str("justification")))
	case "investigation_status":
		return toolResult(s.investigationStatus())
	case "wait_for_message":
		return toolResult(s.waitForMessage(a.str("from"), a.num("timeout_seconds"), a.flag("include_broadcast")))
	case "notebook_read":
		return toolResult(s.notebookRead(a.str("section"), a.num("last"), a.str("search")))
	case "notebook_append":
		return toolResult(s.notebookAppend(a.str("text"), a.str("tag")))
	case "notebook_write_section":
		return toolResult(s.notebookWriteSection(a.str("name"), a.str("text")))
	case "team_status":
		return toolResult(s.teamStatus())
	case "checkpoint":
		return toolResult(s.checkpoint(a.str("summary"), a.str("next_steps")))
	case "claim_lead":
		return toolResult(s.claimLead())
	case "handover_lead":
		return toolResult(s.handoverLead(a.str("to")))
	case "quiz_start":
		return toolResult(s.quizStart(a.str("question")))
	case "quiz_answer":
		return toolResult(s.quizAnswer(a.str("answer")))
	case "quiz_results":
		return toolResult(s.quizResults(a.flag("wait")))
	case "team_exclude":
		return toolResult(s.teamExclude(a.str("agent"), a.str("reason")))
	case "board_add":
		return toolResult(s.boardAddWith(a.str("title"), a.str("details"), a.str("level"), a.str("for"), a.list("requires")))
	case "find_capability":
		return toolResult(s.findCapability(a.str("capability")))
	case "consult_start":
		return toolResult(s.consultStart(a.str("text"), a.num("minutes")))
	case "consult_reply":
		return toolResult(s.consultReply(a.str("id"), a.str("text")))
	case "consult_results":
		return toolResult(s.consultResults(a.str("id"), a.flag("wait")))
	case "board_list":
		return toolResult(s.boardList())
	case "board_claim":
		return toolResult(s.boardClaim(a.str("id")))
	case "board_done":
		return toolResult(s.boardDone(a.str("id"), a.str("result")))
	case "board_release":
		return toolResult(s.boardRelease(a.str("id"), a.str("note")))
	case "delegate":
		return toolResult(s.delegate(a.str("request"), a.str("context")))
	case "delegation_status":
		return toolResult(s.delegationStatus())
	}
	return toolResult("", opErr(ToolCallUnknownTool, "unknown tool %q", name)) // unreachable: validateArgs knows every tool
}

// toolDefs returns the base MCP tools available to every agent.
func toolDefs() []map[string]any {
	obj := func(props map[string]any, req ...string) map[string]any {
		m := map[string]any{"type": "object", "properties": props}
		if len(req) > 0 {
			m["required"] = req
		}
		return m
	}
	str := func(d string) map[string]any { return map[string]any{"type": "string", "description": d} }
	num := func(d string) map[string]any { return map[string]any{"type": "integer", "description": d} }
	tool := func(name, desc string, schema map[string]any) map[string]any {
		return map[string]any{"name": name, "description": desc, "inputSchema": schema}
	}
	return []map[string]any{
		tool("list_agents", "Show active teammates (task, repo, branch, worktree, status) and configured but inactive agents.",
			obj(map[string]any{})),
		tool("send_message", "Send a message to another agent (by name) or to everyone ('all'). Use it to share findings and coordinate work.",
			obj(map[string]any{"to": str("Agent name or 'all'"), "text": str("Message text — concise and specific")}, "to", "text")),
		tool("read_messages", "Read new messages addressed to you and to 'all' (marks them read). With history=N returns the last N messages of all agents without changing read state.",
			obj(map[string]any{"history": num("Optional: how many latest messages to show (history)")})),
		tool("set_status", "Set a short description of what you are doing now (visible to others in list_agents).",
			obj(map[string]any{"status": str("E.g. 'building firmware, flashing the board next'")}, "status")),
		tool("reserve", "Reserve a shared resource (e.g. a debug probe, a serial port, a test host) before using it. The reservation is announced to everyone. With wait_seconds it waits for release (max 600 s).",
			obj(map[string]any{"resource": str("Resource name"), "note": str("Why you are reserving it"), "wait_seconds": num("Seconds to wait if taken (0 = do not wait)")}, "resource")),
		tool("release", "Release a resource you reserved (announced to everyone). Release it right after use.",
			obj(map[string]any{"resource": str("Resource name")}, "resource")),
		tool("list_reservations", "Show current resource reservations.", obj(map[string]any{})),
	}
}

// rpcReq is an incoming JSON-RPC 2.0 message from the MCP client (one line on stdin).
type rpcReq struct {
	ID     json.RawMessage `json:"id"`     // request ID echoed in the response; absent for notifications
	Method string          `json:"method"` // initialize | ping | tools/list | tools/call | notifications/...
	Params json.RawMessage `json:"params"` // method parameters, decoded per method
}

// rpcResult is a successful JSON-RPC 2.0 response.
type rpcResult struct {
	JSONRPC string          `json:"jsonrpc"` // always "2.0"
	ID      json.RawMessage `json:"id"`      // the ID of the request being answered
	Result  any             `json:"result"`  // method-specific result object
}

// rpcError is a JSON-RPC 2.0 error response. Only protocol-level problems use it; failed tool calls are
// returned as normal tools/call results with isError=true, so that the agent can read the message.
type rpcError struct {
	JSONRPC string          `json:"jsonrpc"` // always "2.0"
	ID      json.RawMessage `json:"id"`      // the ID of the request being answered
	Error   map[string]any  `json:"error"`   // {"code": <JSON-RPC error code>, "message": <text>}
}

// JSON-RPC 2.0 error codes used by the server.
const (
	rpcParseError     = -32700 // the line is not valid JSON
	rpcMethodNotFound = -32601 // unknown method
	rpcInvalidParams  = -32602 // the params do not match the method
)

// reply writes a JSON-RPC result. A write failure means the MCP client is gone; it is logged and serve exits
// when stdin closes.
func (s *Server) reply(id json.RawMessage, result any) {
	if err := s.out.Encode(rpcResult{JSONRPC: "2.0", ID: id, Result: result}); err != nil {
		logError(LogJSONRPCWriteFailed, "error", err.Error())
	}
}

// fail writes a JSON-RPC error.
func (s *Server) fail(id json.RawMessage, code int, msg string) {
	if len(id) == 0 {
		id = json.RawMessage("null")
	}
	logWarn(LogJSONRPCError, "code", code, "message", msg)
	if err := s.out.Encode(rpcError{JSONRPC: "2.0", ID: id, Error: map[string]any{"code": code, "message": msg}}); err != nil {
		logError(LogJSONRPCWriteFailed, "error", err.Error())
	}
}

// handle processes one JSON-RPC line (initialize, ping, tools/list, tools/call). Notifications are ignored.
// Every tool result gets an unread-messages footer so agents notice new messages.
func (s *Server) handle(line []byte) {
	var req rpcReq
	if err := json.Unmarshal(line, &req); err != nil {
		s.fail(nil, rpcParseError, "parse error: "+err.Error())
		return
	}
	if len(req.ID) == 0 || string(req.ID) == "null" {
		return // notification (e.g. notifications/initialized) — no response
	}
	switch req.Method {
	case "initialize":
		var p struct {
			ProtocolVersion string `json:"protocolVersion"` // the version the client speaks
		}
		if len(req.Params) > 0 {
			if err := json.Unmarshal(req.Params, &p); err != nil {
				s.fail(req.ID, rpcInvalidParams, "invalid initialize params: "+err.Error())
				return
			}
		}
		pv := p.ProtocolVersion
		if pv == "" {
			pv = defaultProto
		}
		s.reply(req.ID, map[string]any{
			"protocolVersion": pv,
			"capabilities":    map[string]any{"tools": map[string]any{"listChanged": false}},
			"serverInfo":      map[string]any{"name": "agentbus", "version": version},
			"instructions": "Agent communication bus on this host. You are " + s.me +
				". At start call list_agents and read_messages; reserve shared resources before use and release them right after." +
				" A hard problem can be handed to an investigator: request_investigation. Persistent repository knowledge: notebook_read.",
		})
	case "ping":
		s.reply(req.ID, map[string]any{})
	case "tools/list":
		tools := append(toolDefs(), invToolDefs(s.role)...)
		tools = append(tools, capabilityToolDefs()...)
		tools = append(tools, teamToolDefs(s.team != "")...)
		if s.role != RoleInvestigator {
			tools = append(tools, collabToolDefs(s.team != "", s.tier)...)
			tools = append(tools, consultToolDefs(s.team != "")...)
		}
		s.reply(req.ID, map[string]any{"tools": tools})
	case "tools/call":
		var p struct {
			Name      string         `json:"name"`      // tool name
			Arguments map[string]any `json:"arguments"` // tool arguments (validated against the tool's schema)
		}
		if err := json.Unmarshal(req.Params, &p); err != nil {
			s.fail(req.ID, rpcInvalidParams, "invalid tools/call params: "+err.Error())
			return
		}
		text, unread, err := s.callForMCP(p.Name, p.Arguments)
		if err != nil {
			text = "ERROR: " + err.Error()
		}
		if p.Name != "read_messages" {
			n := unread
			var serr *StorageError
			if s.hubClient == nil {
				n, serr = s.unreadCount()
			}
			switch {
			case serr != nil:
				warnSideEffect("count unread messages", serr)
			case n > 0:
				text += fmt.Sprintf("\n\n📬 You have %d unread messages — call read_messages.", n)
			}
		}
		s.reply(req.ID, map[string]any{
			"content": []map[string]any{{"type": "text", "text": text}},
			"isError": err != nil,
		})
	default:
		s.fail(req.ID, rpcMethodNotFound, "method not found: "+req.Method)
	}
}

// serve: runs the MCP server on stdio for one agent run until stdin closes or a signal arrives.
func cmdServe(args []string) {
	fs := newFlagSet("serve")
	dir := fs.String("dir", defaultDir(), "state directory")
	agent := fs.String("agent", "", "this agent's name (required)")
	task := fs.String("task", "", "task ID")
	repo := fs.String("repo", "", "repository name")
	branch := fs.String("branch", "", "agent's branch")
	worktree := fs.String("worktree", "", "path to the agent's worktree")
	peers := fs.String("peers", "", "configured agents, comma-separated")
	role := fs.String("role", "worker", "worker | investigator")
	queueMax := fs.Int("queue-max", 3, "investigation queue limit")
	slots := fs.Int("inv-slots", 1, "number of investigators")
	tasksDir := fs.String("tasks-dir", "", "tasks directory (for investigation files)")
	team := fs.String("team", "", "team ID (team mode)")
	tier := fs.String("tier", "normal", "normal | worker")
	agentCmd := fs.String("agent-cmd", "", "path to the agent script (worker requests)")
	home := fs.String("home", agentsHome(), "AGENTS_HOME")
	parseFlags(fs, args)
	if *agent == "" {
		exitOn(opErr(CmdServeMissingAgent, "--agent is required").asError())
	}
	r, verr := parseEnum[Role](*role, "role")
	exitOn(fromValue(CmdServeInvalidRole, verr).asError())
	tr, verr := parseEnum[Tier](*tier, "tier")
	exitOn(fromValue(CmdServeInvalidTier, verr).asError())
	setupLogging(*home, "serve", "agent", *agent, "task", *task, "team", *team)
	out := json.NewEncoder(os.Stdout)
	out.SetEscapeHTML(false)
	s := &Server{
		bus: &Bus{dir: *dir}, me: *agent, out: out,
		info:  Presence{Agent: *agent, Task: *task, Repo: *repo, Branch: *branch, Worktree: *worktree},
		peers: strings.Split(*peers, ","),
		role:  r, queueMax: *queueMax, tasksDir: *tasksDir, team: *team,
		tier: tr, agentCmd: *agentCmd, home: *home,
	}
	invSlots = *slots
	if s.tasksDir == "" {
		s.tasksDir = filepath.Join(filepath.Dir(*dir), "tasks")
	}
	client, clientErr := hubClientForDir(*dir)
	exitOn(clientErr.asError())
	s.hubClient = client
	if client != nil {
		client.Observe = reachabilityObserver(s.home, client.Instance)
	}
	local, verr := newLocalDirectory(os.Getenv(localInstanceNameEnv), os.Getenv("INSTANCE_CAPABILITIES"))
	exitOn(fromValue(CmdServeInvalidInstanceCapabilities, verr).asError())
	if client != nil {
		// Hub mode: the hub's server resolves the instance and the registry directory, not this process.
		s.hubCaller = HubCaller{Agent: *agent, Instance: client.Instance, Task: *task, Team: *team, Tier: tr, Role: r}
	} else {
		s.instance, s.directory = local.view.Name, local
		exitOn(fromStorage(CmdServeStorage, s.register()).asError())
	}
	logInfo(LogServerStarted, "role", r, "tier", tr, "version", version)
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP)
	go func() {
		got := <-sig
		logInfo(LogServerStopping, "signal", got.String())
		if s.hubClient == nil {
			s.unregister()
		}
		os.Exit(0)
	}()

	in := bufio.NewReader(os.Stdin)
	for {
		line, err := in.ReadBytes('\n')
		if len(bytes.TrimSpace(line)) > 0 {
			s.handle(line)
		}
		if errors.Is(err, io.EOF) {
			logInfo(LogServerStopping, "reason", "stdin closed")
			break
		}
		if err != nil {
			e := &CmdServeError{Cause: CmdServeClientRead, Message: "cannot read from the MCP client: " + err.Error(),
				Err: errors.Join(ErrStorage, err)}
			logError(LogClientReadFailed, "error", e.Error(), "cause", e.CauseName())
			break
		}
	}
	if s.hubClient == nil {
		s.unregister()
	}
}

// ---------------------------------------------------------------- human-facing CLI

// agentsHome is $AGENTS_HOME or ~/agents. Without either there is no sensible default, so the process exits.
func agentsHome() string {
	if d := os.Getenv("AGENTS_HOME"); d != "" {
		return d
	}
	h, err := os.UserHomeDir()
	if err != nil {
		exitOn(&CliError{Cause: CliNoHome, Message: "neither AGENTS_HOME nor a home directory is set: " + err.Error(),
			Err: errors.Join(ErrInvalidArgument, err)})
	}
	return filepath.Join(h, "agents")
}

// defaultDir is $AGENTBUS_DIR or <AGENTS_HOME>/bus.
func defaultDir() string {
	if d := os.Getenv("AGENTBUS_DIR"); d != "" {
		return d
	}
	return filepath.Join(agentsHome(), "bus")
}

// newFlagSet creates the flag set of a subcommand; parse errors are reported by parseFlags.
func newFlagSet(name string) *flag.FlagSet { return flag.NewFlagSet(name, flag.ContinueOnError) }

// parseFlags parses subcommand flags and exits on a parse error (status 2) or after -h (status 0).
func parseFlags(fs *flag.FlagSet, args []string) {
	err := fs.Parse(args)
	if errors.Is(err, flag.ErrHelp) {
		os.Exit(0)
	}
	if err != nil {
		exitOn(&CliError{Cause: CliInvalidFlags, Message: fs.Name() + ": " + err.Error(), Err: errors.Join(ErrInvalidArgument, err)})
	}
}

// fieldFlags collects repeated --field key=value flags.
type fieldFlags map[string]string

// String implements flag.Value.
func (f fieldFlags) String() string { return fmt.Sprint(map[string]string(f)) }

// Set implements flag.Value (which requires the plain error type); it returns a *ValueError.
func (f fieldFlags) Set(v string) error {
	k, val, ok := strings.Cut(v, "=")
	if !ok || k == "" {
		return valueErr("--field must be key=value, got %q", v).asError()
	}
	f[k] = val
	return nil
}

// send: posts a message as a human (chat) or, with --notice, an event of the `agent` script.
func cmdSend(args []string) {
	fs := newFlagSet("send")
	dir := fs.String("dir", defaultDir(), "state directory")
	from := fs.String("from", os.Getenv("USER"), "sender")
	to := fs.String("to", "all", "recipient (agent, all or team:<id>)")
	notice := fs.String("notice", "", "script event kind (see ScriptNotice); empty = chat message")
	fields := fieldFlags{}
	fs.Var(fields, "field", "key=value detail of a script event (repeatable)")
	parseFlags(fs, args)
	text := strings.TrimSpace(strings.Join(fs.Args(), " "))
	if text == "" {
		exitOn(opErr(CmdSendMissingText, "%s", missing("message text")).asError())
	}
	if ok, err := forwardCLI(*dir, "send", map[string]any{"text": text, "from": *from, "to": *to, "notice": *notice, "fields": map[string]string(fields)}); ok {
		exitOn(err)
		return
	}
	var ev busEvent = EventChat{Body: text}
	if *notice != "" {
		n, verr := parseEnum[ScriptNotice](*notice, "script notice")
		exitOn(fromValue(CmdSendInvalidNotice, verr).asError())
		ev = EventScript{Notice: n, Fields: fields, Body: text}
	}
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdSendStorage, func(st *State) *CmdSendError {
		m, serr := b.post(st, *from, *to, ev)
		if serr != nil {
			return fromStorage(CmdSendStorage, serr)
		}
		out.println(fmtMsg(m))
		return fromStorage(CmdSendStorage, out.err)
	}).asError())
}

// log: prints the last N messages.
func cmdLog(args []string) {
	fs := newFlagSet("log")
	dir := fs.String("dir", defaultDir(), "state directory")
	n := fs.Int("n", 30, "how many latest messages")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	msgs, serr := b.messages()
	exitOn(fromStorage(CmdLogStorage, serr).asError())
	start := len(msgs) - *n
	if start < 0 {
		start = 0
	}
	var out cliWriter
	for _, m := range msgs[start:] {
		out.println(fmtMsg(m))
	}
	exitOn(fromStorage(CmdLogStorage, out.err).asError())
}

// status: prints active agents and reservations.
func cmdStatus(args []string) {
	fs := newFlagSet("status")
	dir := fs.String("dir", defaultDir(), "state directory")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdStatusStorage, func(st *State) *CmdStatusError {
		out.println("Active agents:")
		if len(st.Agents) == 0 {
			out.println("  (none)")
		}
		names := make([]string, 0, len(st.Agents))
		for n := range st.Agents {
			names = append(names, n)
		}
		sort.Strings(names)
		for _, n := range names {
			p := st.Agents[n]
			out.printf("  %s — %s (%s) pid %d, status: %s\n", n, p.Task, p.Branch, p.PID, p.Status)
		}
		out.println("Reservations:")
		out.printf("%s", fmtReservations(st))
		return fromStorage(CmdStatusStorage, out.err)
	}).asError())
}

// release: forcibly releases a resource (human override).
func cmdRelease(args []string) {
	fs := newFlagSet("release")
	dir := fs.String("dir", defaultDir(), "state directory")
	parseFlags(fs, args)
	if fs.NArg() != 1 {
		exitOn(opErr(CmdReleaseUsage, "usage: agentbus release [--dir D] <resource>").asError())
	}
	res := fs.Arg(0)
	b := &Bus{dir: *dir}
	var out cliWriter
	exitOn(transact(b, CmdReleaseStorage, func(st *State) *CmdReleaseError {
		r, ok := st.Reservations[res]
		if !ok {
			out.println(res + " is not reserved")
			return fromStorage(CmdReleaseStorage, out.err)
		}
		delete(st.Reservations, res)
		if _, serr := b.post(st, os.Getenv("USER"), "all", EventForcedRelease{Resource: res, Holder: r.Agent}); serr != nil {
			return fromStorage(CmdReleaseStorage, serr)
		}
		out.println("released " + res)
		return fromStorage(CmdReleaseStorage, out.err)
	}).asError())
}

// cliWriter writes the output of a CLI subcommand to stdout and remembers the first write error, which the
// subcommand turns into its storage cause (the `agent` script parses this output).
type cliWriter struct {
	err *StorageError // the first failed write (nil = all writes succeeded)
}

// printf writes formatted output (nothing after a failed write).
func (w *cliWriter) printf(format string, args ...any) {
	if w.err != nil {
		return
	}
	if _, err := fmt.Fprintf(os.Stdout, format, args...); err != nil {
		w.err = storageErr(err, "cannot write to stdout")
	}
}

// println writes a line.
func (w *cliWriter) println(line string) { w.printf("%s\n", line) }

// cmdCommandEnqueue queues a command for a connected node on the hub.
func cmdCommandEnqueue(args []string) {
	fs := newFlagSet("cmd-enqueue")
	dir := fs.String("dir", defaultDir(), "state directory")
	instance := fs.String("instance", "", "target instance")
	kind := fs.String("kind", "", "command kind (start_run, wake, stop_task, answer)")
	task := fs.String("task", "", "task identifier")
	params := make(fieldFlags)
	fs.Var(&params, "param", "key=value parameter (can be repeated)")
	parseFlags(fs, args)
	if *instance == "" || *kind == "" || *task == "" {
		exitOn(opErr(CommandEnqueueInvalidParams, "missing required flags --instance, --kind, or --task").asError())
	}
	if ok, err := forwardCLI(*dir, "command_enqueue", map[string]any{"instance": *instance, "kind": *kind, "task": *task, "params": map[string]string(params)}); ok {
		exitOn(err)
		return
	}
	b := &Bus{dir: *dir}
	h := &Hub{
		Bus:       b,
		Instances: &HubInstanceRegistry{Bus: b},
	}
	err := h.EnqueueCommand(*instance, CommandKind(*kind), *task, params)
	if err != nil {
		exitOn(err.asError())
	}
}

// cmdInstances prints connected instances and their state.
func cmdInstances(args []string) {
	fs := newFlagSet("instances")
	dir := fs.String("dir", defaultDir(), "state directory")
	parseFlags(fs, args)
	if ok, err := forwardCLI(*dir, "instances", nil); ok {
		exitOn(err)
		return
	}
	exitOn(runInstances(args).asError())
}

// runInstances preserves the instances operation on read and stdout failures.
func runInstances(args []string) *CmdInstancesError {
	fs := newFlagSet("instances")
	dir := fs.String("dir", defaultDir(), "state directory")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	reg := &HubInstanceRegistry{Bus: b}
	views, serr := reg.liveViews()
	if serr != nil {
		return fromStorage(CmdInstancesStorage, serr)
	}
	var out cliWriter
	out.printf("%s", formatInstances(views))
	return fromStorage(CmdInstancesStorage, out.err)
}

// exitOn logs the error, prints it for the human and exits with the exit code of its class (see
// ErrorClass.ExitCode). Callers pass typed errors through asError, so a nil pointer never arrives here as a
// non-nil error.
func exitOn(err error) {
	if err == nil {
		return
	}
	class, cause := classOf(err)
	logError(LogCommandFailed, "error", err.Error(), "class", class, "cause", cause)
	fmt.Fprintln(os.Stderr, "agentbus:", err)
	code := 1
	var c classified
	if errors.As(err, &c) {
		code = c.Class().ExitCode()
	}
	os.Exit(code)
}

// main dispatches the subcommand.
func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, `agentbus `+version+`
  agentbus serve   --agent NAME [--dir D --task --repo --branch --worktree --peers a,b]   (MCP server, stdio)
  agentbus hub     [--dir D --listen L --port P --auth A --allowed I]   (multi-instance hub HTTP listener)
  agentbus send    [--dir D] [--from me] [--to all|agent] TEXT
  agentbus log     [--dir D] [-n 30]
  agentbus status  [--dir D]
  agentbus release [--dir D] RESOURCE    (forced release)
  agentbus inv-list [--dir D] [--all]
  agentbus inv-next|inv-set|inv-get|inv-pending|inv-add   (used by the agent script)
  agentbus normalize --type claude|codex|gemini|opencode   (stdin: CLI stream, stdout: activity JSONL)
  agentbus team-list [--all] | notes --repo R [--section S] [-n 30] [--search Q]
  agentbus team-create|team-set|team-done|team-get   (used by the agent script)
  agentbus helper-list [--all] | helper-next|helper-set|helper-get|board-dump   (used by the agent script)
  agentbus checkpoint-get|cancel-pending|board-release-agent   (used by the agent script)`)
		os.Exit(2)
	}
	if os.Args[1] != "serve" { // serve configures logging after parsing its flags (agent, task, team)
		setupLogging(agentsHome(), os.Args[1])
	}
	switch os.Args[1] {
	case "serve":
		cmdServe(os.Args[2:])
	case "node":
		cmdNode(os.Args[2:])
	case "hub":
		cmdHub(os.Args[2:])
	case "cmd-enqueue":
		cmdCommandEnqueue(os.Args[2:])
	case "instances":
		cmdInstances(os.Args[2:])
	case "snapshot":
		exitOn(runSnapshot(os.Args[2:]).asError())
	case "send":
		cmdSend(os.Args[2:])
	case "log":
		cmdLog(os.Args[2:])
	case "status":
		cmdStatus(os.Args[2:])
	case "release":
		cmdRelease(os.Args[2:])
	case "inv-next":
		cmdInvNext(os.Args[2:])
	case "inv-set":
		cmdInvSet(os.Args[2:])
	case "inv-get":
		cmdInvGet(os.Args[2:])
	case "inv-pending":
		cmdInvPending(os.Args[2:])
	case "inv-list":
		cmdInvList(os.Args[2:])
	case "inv-add":
		cmdInvAdd(os.Args[2:])
	case "cancel-pending":
		cmdCancelPending(os.Args[2:])
	case "checkpoint-get":
		cmdCheckpointGet(os.Args[2:])
	case "board-release-agent":
		cmdBoardReleaseAgent(os.Args[2:])
	case "normalize":
		cmdNormalize(os.Args[2:])
	case "team-create":
		cmdTeamCreate(os.Args[2:])
	case "team-set":
		cmdTeamSet(os.Args[2:])
	case "team-done":
		cmdTeamDone(os.Args[2:])
	case "team-get":
		cmdTeamGet(os.Args[2:])
	case "team-list":
		cmdTeamList(os.Args[2:])
	case "notes":
		cmdNotes(os.Args[2:])
	case "helper-next":
		cmdHelperNext(os.Args[2:])
	case "helper-set":
		cmdHelperSet(os.Args[2:])
	case "helper-get":
		cmdHelperGet(os.Args[2:])
	case "helper-list":
		cmdHelperList(os.Args[2:])
	case "board-dump":
		cmdBoardDump(os.Args[2:])
	default:
		exitOn(opErr(CliUnknownSubcommand, "unknown subcommand %q (run agentbus without arguments for the list)", os.Args[1]).asError())
	}
}
