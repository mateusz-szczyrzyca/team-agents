package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/textinput"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------- style

// Styles (adaptive colours for light and dark terminals) and layout constants of the panel.
var (
	stHeader   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.AdaptiveColor{Light: "#1d4ed8", Dark: "#93c5fd"})       // titles and status events
	stDim      = lipgloss.NewStyle().Foreground(lipgloss.AdaptiveColor{Light: "#6b7280", Dark: "#9ca3af"})                  // secondary text
	stSel      = lipgloss.NewStyle().Reverse(true)                                                                          // the selected row
	stWaiting  = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.AdaptiveColor{Light: "#b45309", Dark: "#fbbf24"})       // tasks waiting for the human, reminders
	stRunning  = lipgloss.NewStyle().Foreground(lipgloss.AdaptiveColor{Light: "#047857", Dark: "#6ee7b7"})                  // working tasks
	stInvest   = lipgloss.NewStyle().Foreground(lipgloss.AdaptiveColor{Light: "#7c3aed", Dark: "#c4b5fd"})                  // investigations, bus messages
	stDone     = lipgloss.NewStyle().Foreground(lipgloss.AdaptiveColor{Light: "#15803d", Dark: "#86efac"})                  // finished tasks
	stFailed   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.AdaptiveColor{Light: "#b91c1c", Dark: "#fca5a5"})       // failed/stopped/hung tasks, errors
	stTabOn    = lipgloss.NewStyle().Bold(true).Underline(true)                                                             // the active tab
	stFlash    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.AdaptiveColor{Light: "#b45309", Dark: "#fbbf24"})       // temporary flash messages
	stSection  = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.AdaptiveColor{Light: "#374151", Dark: "#e5e7eb"})       // section headings in the detail view
	tabs       = []string{"1 Activity", "2 Prompt", "3 Questions", "4 Investigations", "5 Result", "6 Notebook", "7 Board"} // detail view tabs (keys 1–7)
	maxTableRw = 8                                                                                                          // maximum number of task rows shown at once (the table scrolls)
)

// statusStyle returns the label and style of a task status.
func statusStyle(s string) (string, lipgloss.Style) {
	switch s {
	case "waiting":
		return "? waiting for you", stWaiting
	case "running":
		return "● working", stRunning
	case "starting", "new":
		return "◌ starting", stRunning
	case "investigating":
		return "⌕ awaiting investigation", stInvest
	case "done":
		return "✓ done", stDone
	case "failed":
		return "✗ failed", stFailed
	case "stopped":
		return "■ stopped", stFailed
	case "sleeping":
		return "zz sleeping (team)", stDim
	case "paused":
		return "⏸ paused", stDim
	case "excluded":
		return "⊘ excluded", stFailed
	case "hung":
		return "⚠ hung", stFailed
	}
	return s, stDim
}

// eventStyle returns the style of an event kind.
func eventStyle(k string) lipgloss.Style {
	switch k {
	case "status":
		return stHeader
	case "err":
		return stFailed
	case "remind":
		return stWaiting
	case "bus":
		return stInvest
	case "act":
		return stDim
	}
	return lipgloss.NewStyle()
}

// fmtEvent renders an event line with its timestamp.
func fmtEvent(e Event) string {
	return stDim.Render(e.TS.Format("15:04:05")) + " " + eventStyle(e.Kind).Render(e.Text)
}

// ---------------------------------------------------------------- model

// mode is the panel's input mode (what keys do).
type mode int

const (
	modeNormal      mode = iota // browsing: navigation and single-key actions
	modeConfirmStop             // waiting for y/n after `s` (stop the selected task)
	modeInput                   // typing a message (`m` / `M`)
	modeFeed                    // full-screen event stream (`f`)
)

// tickMsg triggers the periodic (one second) refresh.
type tickMsg time.Time

// flashMsg is a short message shown in the help line (e.g. the output of an `agent` action).
type flashMsg string

// model is the Bubble Tea model of the panel.
type model struct {
	cfg      config          // command-line settings
	w        *Watcher        // produces snapshots and events
	snap     *Snapshot       // the latest snapshot
	events   []Event         // recent events (bottom pane and the feed)
	selID    string          // ID of the selected task
	tab      int             // selected detail tab (0-based)
	vp       viewport.Model  // detail view
	feed     viewport.Model  // full-screen event stream
	follow   bool            // keep the detail view scrolled to the bottom (activity tab)
	mode     mode            // input mode
	input    textinput.Model // message input (modeInput)
	target   string          // recipient of the message being typed (agent name or "all")
	flash    string          // temporary message in the help line
	flashAt  time.Time       // when flash was set (it expires after a few seconds)
	width    int             // terminal width
	height   int             // terminal height
	lastKey  string          // "<task>/<tab>" shown in the detail view; a change resets the scroll position
	stopping string          // task ID waiting for the stop confirmation (modeConfirmStop)
}

// newModel creates the Bubble Tea model with an initial snapshot.
func newModel(cfg config) model {
	ti := textinput.New()
	ti.Prompt = "› "
	ti.CharLimit = 2000
	w := NewWatcher(cfg.home, cfg.busDir, cfg.remind)
	snap, _ := w.Poll() // the first poll only records the baseline: it has no events
	m := model{cfg: cfg, w: w, snap: snap, input: ti, follow: true,
		vp: viewport.New(80, 10), feed: viewport.New(80, 20)}
	if len(snap.Tasks) > 0 {
		m.selID = snap.Tasks[0].ID
	}
	m.events = append(m.events, Event{time.Now(), "info", "agent live — " + snap.QueueLine(cfg.queueMax)})
	if r := remindText(snap); r != "" {
		m.events = append(m.events, Event{time.Now(), "remind", r})
	}
	return m
}

// tick schedules the next one-second refresh.
func tick() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg { return tickMsg(t) })
}

// Init starts the refresh ticker.
func (m model) Init() tea.Cmd { return tick() }

// selected returns the selected task (keeping the selection valid when tasks disappear).
func (m *model) selected() *Task {
	for _, t := range m.snap.Tasks {
		if t.ID == m.selID {
			return t
		}
	}
	if len(m.snap.Tasks) > 0 {
		m.selID = m.snap.Tasks[0].ID
		return m.snap.Tasks[0]
	}
	m.selID = ""
	return nil
}

// moveSel moves the selection by d rows (wrapping around).
func (m *model) moveSel(d int) {
	n := len(m.snap.Tasks)
	if n == 0 {
		return
	}
	idx := 0
	for i, t := range m.snap.Tasks {
		if t.ID == m.selID {
			idx = i
		}
	}
	idx = (idx + d + n) % n
	m.selID = m.snap.Tasks[idx].ID
	m.follow = true
}

// setFlash shows a temporary message in the help line.
func (m *model) setFlash(s string) { m.flash, m.flashAt = s, time.Now() }

// runAgent runs an `agent` subcommand in the background and reports its output as a flash message.
func runAgent(cfg config, args ...string) tea.Cmd {
	return func() tea.Msg {
		out, err := exec.Command(cfg.agentCmd, args...).CombinedOutput() // #nosec G204 -- cfg.agentCmd is the operator's configured agent command; args are passed as separate argv entries, never through a shell
		msg := strings.TrimSpace(string(out))
		if err != nil && msg == "" {
			msg = err.Error()
		}
		return flashMsg(trunc(msg, 200))
	}
}

// Update handles window size, ticks, flash messages and keys (per input mode).
func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		m.layout()
		m.refreshDetail()
		return m, nil

	case tickMsg:
		snap, ev := m.w.Poll()
		m.snap = snap
		m.events = append(m.events, ev...)
		if len(m.events) > 2000 {
			m.events = m.events[len(m.events)-2000:]
		}
		m.selected()
		m.layout()
		m.refreshDetail()
		if m.mode == modeFeed {
			m.refreshFeed()
		}
		return m, tick()

	case flashMsg:
		m.setFlash(string(msg))
		return m, nil

	case tea.KeyMsg:
		switch m.mode {
		case modeInput:
			switch msg.String() {
			case "esc":
				m.mode = modeNormal
				return m, nil
			case "enter":
				text := strings.TrimSpace(m.input.Value())
				m.mode = modeNormal
				if text == "" {
					return m, nil
				}
				return m, runAgent(m.cfg, "say", m.target, text)
			}
			var cmd tea.Cmd
			m.input, cmd = m.input.Update(msg)
			return m, cmd
		case modeConfirmStop:
			m.mode = modeNormal
			if strings.ToLower(msg.String()) == "t" || msg.String() == "y" {
				return m, runAgent(m.cfg, "stop", m.stopping)
			}
			m.setFlash("cancelled")
			return m, nil
		case modeFeed:
			switch msg.String() {
			case "f", "esc":
				m.mode = modeNormal
				return m, nil
			case "q", "ctrl+c":
				return m, tea.Quit
			case "pgup":
				m.feed.ViewUp()
			case "pgdown":
				m.feed.ViewDown()
			case "up":
				m.feed.LineUp(1)
			case "down":
				m.feed.LineDown(1)
			case "end", "G":
				m.feed.GotoBottom()
			}
			return m, nil
		}
		return m.handleKey(msg)
	}
	return m, nil
}

// handleKey handles keys in normal mode.
func (m model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	t := m.selected()
	switch k := msg.String(); k {
	case "q", "ctrl+c":
		return m, tea.Quit
	case "up", "k":
		m.moveSel(-1)
	case "down", "j":
		m.moveSel(1)
	case "1", "2", "3", "4", "5", "6", "7":
		m.tab = int(k[0] - '1')
		m.follow = true
		m.lastKey = ""
	case "tab":
		m.tab = (m.tab + 1) % len(tabs)
		m.follow = true
		m.lastKey = ""
	case "pgup":
		m.vp.ViewUp()
		m.follow = false
	case "pgdown":
		m.vp.ViewDown()
		m.follow = m.vp.AtBottom()
	case "home":
		m.vp.GotoTop()
		m.follow = false
	case "end":
		m.vp.GotoBottom()
		m.follow = true
	case "f":
		m.mode = modeFeed
		m.refreshFeed()
		m.feed.GotoBottom()
		return m, nil
	case "a":
		if t == nil || t.Status != "waiting" {
			m.setFlash("this task is not waiting for your answers")
			return m, nil
		}
		c := exec.Command(m.cfg.agentCmd, "answer", t.ID) // #nosec G204 -- m.cfg.agentCmd is the operator's configured agent command; t.ID comes from the task listing
		return m, tea.ExecProcess(c, func(err error) tea.Msg {
			if err != nil {
				return flashMsg("agent answer: " + err.Error())
			}
			return flashMsg("back from answering questions (" + t.Label() + ")")
		})
	case "s":
		if t == nil || !t.Active() {
			m.setFlash("this task is not running")
			return m, nil
		}
		m.mode, m.stopping = modeConfirmStop, t.ID
	case "m", "M":
		m.target = "all"
		if k == "m" && t != nil {
			m.target = t.BusName()
		}
		m.input.SetValue("")
		m.input.Placeholder = "message to " + m.target + " (Enter sends, Esc cancels)"
		m.input.Focus()
		m.mode = modeInput
		return m, textinput.Blink
	case "x":
		if t == nil || t.Active() {
			m.setFlash("only finished tasks can be acknowledged")
			return m, nil
		}
		if err := os.WriteFile(filepath.Join(t.Dir, "acked"), nil, 0o600); err != nil {
			logWarn(LogAcknowledgeFailed, "task", t.ID, "error", storageErr(err, "cannot write %s", t.Dir+"/acked").Error())
			m.setFlash("cannot acknowledge " + t.Label() + ": " + err.Error())
			return m, nil
		}
		m.setFlash("acknowledged " + t.Label())
	case "X":
		n, failed := 0, 0
		for _, tt := range m.snap.Tasks {
			if tt.Active() {
				continue
			}
			if err := os.WriteFile(filepath.Join(tt.Dir, "acked"), nil, 0o600); err != nil {
				logWarn(LogAcknowledgeFailed, "task", tt.ID, "error", storageErr(err, "cannot write %s", tt.Dir+"/acked").Error())
				failed++
				continue
			}
			n++
		}
		if failed > 0 {
			m.setFlash(fmt.Sprintf("acknowledged finished: %d, failed: %d (see logs/agentlive.jsonl)", n, failed))
		} else {
			m.setFlash(fmt.Sprintf("acknowledged finished: %d", n))
		}
	}
	m.refreshDetail()
	return m, nil
}

// ---------------------------------------------------------------- layout and content

// layout computes the heights of the detail and feed viewports.
func (m *model) layout() {
	rows := len(m.snap.Tasks)
	if rows > maxTableRw {
		rows = maxTableRw
	}
	if rows == 0 {
		rows = 1
	}
	used := 1 + 1 + rows + 1 + 1 + 7 + 1 + 1 // header, table (header+rows), tabs, separator, events, help line
	h := m.height - used
	if h < 3 {
		h = 3
	}
	m.vp.Width, m.vp.Height = m.width, h
	m.feed.Width, m.feed.Height = m.width, m.height-2
}

// readFile returns a file's content ("" when it does not exist; other read errors are logged once).
func readFile(path string) string {
	data, err := os.ReadFile(path) // #nosec G304 -- the path is a bus or task file the program lists itself; no request text reaches it
	if errors.Is(err, os.ErrNotExist) {
		return "" // the file does not exist yet (no questions, no result)
	}
	if err != nil {
		warnOnce("read:"+path, LogFileReadFailed, storageErr(err, "cannot read %s", path), "file", path)
		return ""
	}
	return string(data)
}

// detail renders the selected tab for a task.
func (m *model) detail(t *Task) string {
	w := m.width
	if w < 20 {
		w = 80
	}
	wrap := lipgloss.NewStyle().Width(w)
	switch m.tab {
	case 0: // activity
		var b strings.Builder
		acts := readActivity(t.Dir, 400)
		if len(acts) == 0 {
			return stDim.Render("(no activity — task from an older version or not started yet)")
		}
		for _, a := range acts {
			ts := shortTS(a.TS)
			line := stDim.Render(ts) + " " + actIcon(a.Kind) + " " + a.Text
			switch a.Kind {
			case "err":
				line = stDim.Render(ts) + " " + stFailed.Render(actIcon(a.Kind)+" "+a.Text)
			case "info":
				line = stDim.Render(ts + " " + actIcon(a.Kind) + " " + a.Text)
			case "text":
				line = stDim.Render(ts) + " " + stSection.Render(actIcon(a.Kind)) + " " + a.Text
			}
			b.WriteString(wrap.Render(line) + "\n")
		}
		return b.String()
	case 1: // prompt
		p := filepath.Join(t.Dir, fmt.Sprintf("prompt-%d.md", t.Run))
		c := readFile(p)
		if c == "" {
			return stDim.Render("(no prompt)")
		}
		return stDim.Render(p) + "\n\n" + wrap.Render(c)
	case 2: // questions
		if t.Round == 0 {
			return stDim.Render("(the agent asked no questions)")
		}
		var b strings.Builder
		for r := t.Round; r >= 1; r-- {
			p := filepath.Join(t.Dir, fmt.Sprintf("questions-%d.md", r))
			label := fmt.Sprintf("— round %d", r)
			if r == t.Round && t.Status == "waiting" {
				label += " (WAITING FOR ANSWERS — key a)"
			}
			b.WriteString(stSection.Render(label) + "  " + stDim.Render(p) + "\n\n" + wrap.Render(readFile(p)) + "\n\n")
		}
		return b.String()
	case 3: // investigations
		var b strings.Builder
		for _, inv := range m.snap.Invs {
			if inv.FromTask != t.ID && (t.Kind != "investigation" || fmt.Sprint(inv.ID) != t.InvID) {
				continue
			}
			b.WriteString(stSection.Render(fmt.Sprintf("#%d [%s]", inv.ID, inv.Status)) + " " + inv.Question + "\n")
			if c := readFile(inv.File); c != "" {
				b.WriteString(wrap.Render(c) + "\n")
			}
			b.WriteString(strings.Repeat("─", min(w, 60)) + "\n")
		}
		if b.Len() == 0 {
			return stDim.Render("(no investigations for this task)")
		}
		return b.String()
	case 6: // team board and quiz
		ti := m.snap.Teams[t.Team]
		if t.Team == "" || ti == nil {
			return stDim.Render("(this task is not part of a team)")
		}
		var b strings.Builder
		if q := ti.Quiz; q != nil {
			b.WriteString(stSection.Render("Quiz") + stDim.Render("  ("+q.AskedAt+")") + "\n" + wrap.Render(q.Question) + "\n")
			names := make([]string, 0, len(q.Answers))
			for n := range q.Answers {
				names = append(names, n)
			}
			sort.Strings(names)
			for _, n := range names {
				a := q.Answers[n]
				fmt.Fprintf(&b, "  %s: %q after %d s\n", n, a.Answer, a.Secs)
			}
			b.WriteString("\n")
		}
		if len(ti.Members) > 0 {
			b.WriteString(stSection.Render("Members") + "\n")
			names := make([]string, 0, len(ti.Members))
			for n := range ti.Members {
				names = append(names, n)
			}
			sort.Strings(names)
			for _, n := range names {
				mm := ti.Members[n]
				lead := ""
				if n == ti.Lead {
					lead = " ★ coordinator"
				}
				fmt.Fprintf(&b, "  %s [%s] %s%s\n", n, mm.Tier, mm.State, lead)
			}
			b.WriteString("\n")
		}
		b.WriteString(stSection.Render("Task board") + "\n")
		ids := make([]string, 0, len(ti.Board))
		for id := range ti.Board {
			ids = append(ids, id)
		}
		sort.Slice(ids, func(i, j int) bool {
			if len(ids[i]) != len(ids[j]) {
				return len(ids[i]) < len(ids[j])
			}
			return ids[i] < ids[j]
		})
		if len(ids) == 0 {
			b.WriteString(stDim.Render("(empty)") + "\n")
		}
		for _, id := range ids {
			it := ti.Board[id]
			who := ""
			if it.Assignee != "" {
				who = " → " + it.Assignee
			}
			st := stDim
			switch it.Status {
			case "claimed":
				st = stRunning
			case "done":
				st = stDone
			}
			b.WriteString(st.Render(fmt.Sprintf("%s [%s, %s%s]", id, it.Level, it.Status, who)) + " " + it.Title + stDim.Render(" (from "+it.Creator+")") + "\n")
			if it.Status == "done" && it.Result != "" {
				b.WriteString(wrap.Render("    result: "+it.Result) + "\n")
			} else if it.Details != "" {
				b.WriteString(wrap.Render("    "+it.Details) + "\n")
			}
		}
		return b.String()
	case 5: // repository notebook
		secs, notes := Notebook(m.cfg.busDir, t.Repo, 200)
		var b strings.Builder
		b.WriteString(stSection.Render("Repository notebook "+t.Repo) + stDim.Render("  (agent notes "+t.Repo+")") + "\n\n")
		names := make([]string, 0, len(secs))
		for n := range secs {
			names = append(names, n)
		}
		sort.Strings(names)
		for _, n := range names {
			sec := secs[n]
			b.WriteString(stSection.Render("## "+n) + stDim.Render("  ("+sec.Author+", "+sec.Updated+")") + "\n" + wrap.Render(sec.Text) + "\n\n")
		}
		b.WriteString(stSection.Render("Log") + "\n")
		if len(notes) == 0 {
			b.WriteString(stDim.Render("(empty)") + "\n")
		}
		for _, e := range notes {
			tag := ""
			if e.Tag != "" {
				tag = " [" + e.Tag + "]"
			}
			b.WriteString(wrap.Render(stDim.Render(e.TS+" "+e.Author+tag)+" "+e.Text) + "\n")
		}
		return b.String()
	default: // result
		var b strings.Builder
		if c := readFile(filepath.Join(t.Dir, "final.md")); strings.TrimSpace(c) != "" {
			b.WriteString(stSection.Render("Agent's last message") + "\n" + wrap.Render(c) + "\n\n")
		}
		if t.Kind == "investigation" {
			if c := readFile(filepath.Join(t.Dir, "result.md")); c != "" {
				b.WriteString(stSection.Render("result.md") + "\n" + wrap.Render(c) + "\n")
			}
		} else {
			rp := filepath.Join(t.Worktree, ".agents", "reports", t.ID+".md")
			if c := readFile(rp); c != "" {
				b.WriteString(stSection.Render("Report "+rp) + "\n" + wrap.Render(c) + "\n")
			}
		}
		if b.Len() == 0 {
			return stDim.Render("(no result yet)")
		}
		return b.String()
	}
}

// refreshDetail re-renders the detail viewport, keeping the scroll position unless following.
func (m *model) refreshDetail() {
	t := m.selected()
	if t == nil {
		m.vp.SetContent(stDim.Render("No active tasks. Start one: agent run <agent> \"task\""))
		return
	}
	key := fmt.Sprintf("%s/%d", t.ID, m.tab)
	if key != m.lastKey {
		m.follow = m.tab == 0 // the activity tab follows the end, the others start at the top
		m.lastKey = key
		m.vp.SetContent(m.detail(t))
		if m.follow {
			m.vp.GotoBottom()
		} else {
			m.vp.GotoTop()
		}
		return
	}
	m.vp.SetContent(m.detail(t))
	if m.follow {
		m.vp.GotoBottom()
	}
}

// refreshFeed re-renders the full-screen event stream.
func (m *model) refreshFeed() {
	var b strings.Builder
	for _, e := range m.events {
		b.WriteString(lipgloss.NewStyle().Width(m.width).Render(fmtEvent(e)) + "\n")
	}
	atBottom := m.feed.AtBottom()
	m.feed.SetContent(b.String())
	if atBottom {
		m.feed.GotoBottom()
	}
}

// row renders one task row of the table.
func (m model) row(t *Task, width int) string {
	label, st := statusStyle(t.Status)
	var since string
	switch {
	case t.Active() && t.Alive && !t.RunStart.IsZero():
		since = human(time.Since(t.RunStart))
	case t.Active():
		since = human(time.Since(t.Created))
	case !t.Finished.IsZero():
		since = "-" + human(time.Since(t.Finished))
	}
	stage := ""
	if p := m.snap.Agents[t.BusName()]; p != nil && p.Task == t.ID && p.Status != "" {
		stage = "„" + p.Status + "” "
	}
	if (t.Status == "running" || t.Status == "starting") && !t.Alive && !t.RunStart.IsZero() {
		label = "✗ process is dead"
		st = stFailed
	}
	last := ""
	switch {
	case !t.Active():
		last = finalSummary(t.Dir)
	case t.Last.Text != "":
		last = shortTS(t.Last.TS) + " " + actIcon(t.Last.Kind) + " " + t.Last.Text
	}
	if t.Kind == "investigation" {
		stage = "for " + t.FromAgent + " " + stage
	}
	if t.Team != "" {
		stage = "[team " + strings.TrimSuffix(t.Team, "-team") + "] " + stage
	}
	name := t.Label()
	if t.Team != "" {
		if ti := m.snap.Teams[t.Team]; ti != nil && ti.Lead == t.Agent {
			name += "★"
		} else {
			name += "·"
		}
	}
	head := fmt.Sprintf("%-11s ", trunc(name, 11))
	stat := st.Render(fmt.Sprintf("%-19s", label))
	rest := fmt.Sprintf(" %6s  %-10s %s%s", since, trunc(t.Repo, 10), stage, last)
	line := head + stat + rest
	return lipgloss.NewStyle().MaxWidth(width).Render(line)
}

// View renders the whole screen.
func (m model) View() string {
	if m.width == 0 {
		return "…"
	}
	if m.mode == modeFeed {
		help := stDim.Render("event stream — ↑↓ PgUp/PgDn scroll, End bottom, f/Esc back, q quit")
		return m.feed.View() + "\n" + help
	}
	var b strings.Builder
	waiting := len(m.snap.Waiting())
	head := stHeader.Render("agent live") + "  " + stDim.Render(m.snap.At.Format("15:04:05")) + "  │ " + m.snap.QueueLine(m.cfg.queueMax)
	if waiting > 0 {
		head += "  │ " + stWaiting.Render(fmt.Sprintf("waiting for you: %d", waiting))
	}
	b.WriteString(lipgloss.NewStyle().MaxWidth(m.width).Render(head) + "\n")
	b.WriteString(stDim.Render(lipgloss.NewStyle().MaxWidth(m.width).Render(
		fmt.Sprintf("%-11s %-19s %6s  %-10s %s", "AGENT", "STATUS", "TIME", "REPO", "STAGE / LAST ACTIVITY"))) + "\n")

	tasks := m.snap.Tasks
	start := 0
	for i, t := range tasks {
		if t.ID == m.selID && i >= maxTableRw {
			start = i - maxTableRw + 1
		}
	}
	shown := 0
	for i := start; i < len(tasks) && shown < maxTableRw; i++ {
		line := m.row(tasks[i], m.width-2)
		if tasks[i].ID == m.selID {
			b.WriteString(stSel.Render("›") + " " + line + "\n")
		} else {
			b.WriteString("  " + line + "\n")
		}
		shown++
	}
	if len(tasks) == 0 {
		b.WriteString(stDim.Render("  (no tasks)") + "\n")
	} else if len(tasks) > maxTableRw {
		b.WriteString(stDim.Render(fmt.Sprintf("  … %d tasks in total", len(tasks))) + "\n")
	}

	var tb []string
	for i, name := range tabs {
		if i == m.tab {
			tb = append(tb, stTabOn.Render(name))
		} else {
			tb = append(tb, stDim.Render(name))
		}
	}
	title := ""
	if t := m.selected(); t != nil {
		title = stSection.Render(t.Label()) + stDim.Render(" "+t.ID+" · "+t.Type+" "+t.Model+" · "+t.Branch) + "   "
	}
	b.WriteString(lipgloss.NewStyle().MaxWidth(m.width).Render(title+strings.Join(tb, "  ")) + "\n")
	b.WriteString(m.vp.View() + "\n")
	b.WriteString(stDim.Render(strings.Repeat("─", m.width)) + "\n")

	ev := m.events
	if len(ev) > 7 {
		ev = ev[len(ev)-7:]
	}
	for i := 0; i < 7; i++ {
		if i < len(ev) {
			b.WriteString(lipgloss.NewStyle().MaxWidth(m.width).Render(fmtEvent(ev[i])))
		}
		b.WriteString("\n")
	}

	switch {
	case m.mode == modeInput:
		b.WriteString(m.input.View())
	case m.mode == modeConfirmStop:
		b.WriteString(stFlash.Render("Stop " + m.stopping + "? (t/n)"))
	case m.flash != "" && time.Since(m.flashAt) < 6*time.Second:
		b.WriteString(stFlash.Render(lipgloss.NewStyle().MaxWidth(m.width).Render(m.flash)))
	default:
		b.WriteString(stDim.Render(lipgloss.NewStyle().MaxWidth(m.width).Render(
			"↑↓ task  1-7/Tab tab  PgUp/PgDn/End scroll  a answer  s stop  m message (M to all)  x acknowledge (X all)  f stream  q quit")))
	}
	return b.String()
}
