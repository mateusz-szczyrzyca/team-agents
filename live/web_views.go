// The views of the web interface: one handler per view, each building the view's data from a fresh snapshot
// (loadSnapshot, the same reader as the panel), and the live streams (server-sent events) that keep the dashboard,
// a task's activity and the bus up to date.
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"html/template"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

// webFuncs are the template functions.
var webFuncs = template.FuncMap{
	"statusLabel": func(s string) string { l, _ := statusStyle(s); return l },
	"statusClass": statusClass,
	"actIcon":     actIcon,
	"shortTS":     shortTS,
	"trunc":       trunc,
	"lower":       strings.ToLower,
	"join":        strings.Join,
	"inc":         func(i int) int { return i + 1 },
	"base":        filepath.Base,
	"split":       strings.Fields,
}

// statusClass is the CSS class of a task status (or a member state).
func statusClass(s string) string {
	switch s {
	case "waiting":
		return "st-waiting"
	case "running", "starting", "new", "active", "waking":
		return "st-running"
	case "investigating":
		return "st-invest"
	case "done":
		return "st-done"
	case "stopped":
		return "st-stopped"
	case "failed", "excluded", "hung", "dead":
		return "st-failed"
	case "offline":
		return "st-offline"
	}
	return "st-dim"
}

// snapshot reads the current state (tasks and the bus).
func (s *webServer) snapshot() *Snapshot {
	return loadSnapshot(s.cfg.home, s.cfg.busDir, s.cfg.nodeTTL)
}

// ---------------------------------------------------------------- task rows (the panel's table)

// taskRow is a task as the tables show it — the columns of the panel.
type taskRow struct {
	ID          string    // task ID
	Name        string    // agent, inv#N or agent/HN
	Status      string    // task status
	StatusLabel string    // the status as the panel shows it
	StatusClass string    // CSS class of the status
	Since       string    // time in the current run, or since it finished ("-5m")
	Repo        string    // repository
	Team        string    // team ID ("" for a solo task)
	TeamShort   string    // team ID without the "-team" suffix
	Lead        bool      // the task's agent coordinates its team
	JobMarker   string    // remote job marker ("job <id> from <lead>")
	Stage       string    // set_status text and the "for <agent>" of investigations
	Last        string    // the latest activity (active tasks) or the final summary
	LastIcon    string    // the icon of the latest activity
	Running     bool      // a run is going on (the dashboard offers to stop it)
	Link        string    // where the name links: the task's page, or a remote job's page
	FinishedAt  time.Time // when the task or remote job finished (zero for active)
}

// makeRow builds a task's row exactly as the panel does (ui.go: row).
// dashRecent is how long a finished task stays on the dashboard even when acknowledged — investigations are
// acknowledged as soon as their result is delivered.
const dashRecent = 24 * time.Hour

// dashTasks are the dashboard's tasks: the active ones in the panel's order, then the finished ones — not yet
// acknowledged, or finished within dashRecent.
func dashTasks(snap *Snapshot) ([]*Task, []*Task) {
	var active, done []*Task
	for _, t := range snap.All {
		switch {
		case t.Active():
			active = append(active, t)
		case !t.Acked || time.Since(finishedAt(t)) < dashRecent:
			done = append(done, t)
		}
	}
	sort.SliceStable(active, func(i, j int) bool {
		if pi, pj := statusPrio(active[i]), statusPrio(active[j]); pi != pj {
			return pi < pj
		}
		return active[i].ID < active[j].ID
	})
	return active, done
}

// finishedAt is when a finished task ended (its last activity if FINISHED is missing).
func finishedAt(t *Task) time.Time {
	if !t.Finished.IsZero() {
		return t.Finished
	}
	return lastActivity(t)
}

func makeRow(snap *Snapshot, t *Task) taskRow {
	r := taskRow{ID: t.ID, Name: t.Label(), Status: t.Status, Repo: t.Repo, Team: t.Team, TeamShort: strings.TrimSuffix(t.Team, "-team"), JobMarker: t.JobMarker(), FinishedAt: finishedAt(t)}
	r.Link = "/task/" + t.ID
	r.StatusLabel, _ = statusStyle(t.Status)
	r.StatusClass = statusClass(t.Status)
	r.Running = t.Node == "" && (t.Status == "running" || t.Status == "starting")
	if t.Kind == "investigation" {
		r.Name = fmt.Sprintf("investigator #%s (%s)", t.InvID, t.Agent)
	}
	switch {
	case t.Active() && t.Alive && !t.RunStart.IsZero():
		r.Since = human(time.Since(t.RunStart))
	case t.Active():
		r.Since = human(time.Since(t.Created))
	case !t.Finished.IsZero():
		r.Since = human(time.Since(t.Finished)) + " ago"
	}
	if p := snap.Agents[t.BusName()]; p != nil && p.Task == t.ID && p.Status != "" {
		r.Stage = "„" + p.Status + "”"
	}
	if t.Node == "" && (t.Status == "running" || t.Status == "starting") && !t.Alive && !t.RunStart.IsZero() {
		r.StatusLabel, r.StatusClass = "✗ process is dead", statusClass("dead")
	}
	if t.NodeOffline {
		r.StatusLabel += " (" + t.Status + "; node offline)"
		r.StatusClass = statusClass("offline")
	}
	switch {
	case !t.Active():
		r.Last = finalSummary(t.Dir)
	case t.Last.Text != "":
		r.Last, r.LastIcon = shortTS(t.Last.TS)+" "+t.Last.Text, actIcon(t.Last.Kind)
	}
	if t.Kind == "investigation" {
		r.Stage = strings.TrimSpace("for " + t.FromAgent + " " + r.Stage)
	}
	if ti := snap.Teams[t.Team]; ti != nil && ti.Lead == t.Agent {
		r.Lead = true
	}
	return r
}

// ---------------------------------------------------------------- dashboard

// dashData is the dashboard: the panel's header line, table and events, plus the reservations.
type dashData struct {
	Queue   string         // the investigation queue line
	Waiting int            // tasks waiting for answers
	Rows    []taskRow      // the visible tasks, in the panel's order
	Res     []*Reservation // reserved resources
}

// dashboard builds the dashboard's data.
func (s *webServer) dashboard(snap *Snapshot) dashData {
	d := dashData{Queue: snap.QueueLine(s.cfg.queueMax), Waiting: len(snap.Waiting()), Res: snap.Res}
	active, done := dashTasks(snap)

	var localActiveRows, localDoneRows []taskRow
	for _, t := range active {
		localActiveRows = append(localActiveRows, makeRow(snap, t))
	}
	for _, t := range done {
		localDoneRows = append(localDoneRows, makeRow(snap, t))
	}

	remRows := remoteRows(s.cfg.busDir)
	var remoteActive, remoteDone []taskRow
	for _, r := range remRows {
		hasProxy := false
		for _, t := range snap.All {
			if t.JobID == r.ID {
				hasProxy = true
				break
			}
		}
		if hasProxy {
			continue
		}
		if r.Status == "done" || r.Status == "failed" || r.Status == "stopped" {
			remoteDone = append(remoteDone, r)
		} else {
			remoteActive = append(remoteActive, r)
		}
	}

	doneRows := append(localDoneRows, remoteDone...)
	sort.SliceStable(doneRows, func(i, j int) bool {
		if !doneRows[i].FinishedAt.Equal(doneRows[j].FinishedAt) {
			return doneRows[i].FinishedAt.After(doneRows[j].FinishedAt)
		}
		return doneRows[i].ID > doneRows[j].ID
	})

	d.Rows = append(remoteActive, localActiveRows...)
	d.Rows = append(d.Rows, doneRows...)
	return d
}

// viewDashboard (GET /) is the main page: the same data as `agent live`.
func (s *webServer) viewDashboard(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	s.render(w, r, http.StatusOK, "dashboard", "Dashboard", snap, s.dashboard(snap))
}

// eventRow is one event of the dashboard's event list (the panel's events pane).
type eventRow struct {
	Time  string // HH:MM:SS
	Kind  string // status | bus | act | info | err
	Text  string // the event
	Class string // CSS class
}

// streamDashboard (GET /events/dashboard) sends the table when it changes, the header line and new events.
func (s *webServer) streamDashboard(w http.ResponseWriter, r *http.Request) {
	f, ok := sseStream(w)
	if !ok {
		return
	}
	watcher := NewWatcher(s.cfg.home, s.cfg.busDir, 0)
	watcher.Poll() // the baseline: events are what happens from now on
	tk := time.NewTicker(s.cfg.poll)
	defer tk.Stop()
	var lastTable, lastRes string
	for {
		select {
		case <-r.Context().Done():
			return
		case <-tk.C:
		}
		snap, evs := watcher.Poll()
		d := s.dashboard(snap)
		if table := s.fragment("dash-table", d); table != lastTable {
			if !sseSend(w, f, "table", table) {
				return
			}
			lastTable = table
		}
		if res := s.fragment("dash-res", d); res != lastRes {
			if !sseSend(w, f, "res", res) {
				return
			}
			lastRes = res
		}
		if !sseSend(w, f, "head", s.fragment("dash-head", d)) {
			return
		}
		for _, e := range evs {
			ev := eventRow{Time: e.TS.Format("15:04:05"), Kind: e.Kind, Text: e.Text, Class: "ev-" + e.Kind}
			if !sseSend(w, f, "event", s.fragment("dash-event", ev)) {
				return
			}
		}
	}
}

// ---------------------------------------------------------------- agents

// agentRow is one configured agent with what it is doing now.
type agentRow struct {
	agentEntry           // the agent as configured
	State      string    // free | the status of its most urgent active task
	StateClass string    // CSS class of the state
	Tasks      []taskRow // its active tasks
	Presence   string    // its set_status text
	Paused     string    // "until HH:MM (reason)" when paused
	LastAt     time.Time // its latest activity in any task (zero: none)
	Last       string    // LastAt as shown
}

// lastActivity is the latest moment a task shows activity: its run's start, its end or its latest activity entry.
func lastActivity(t *Task) time.Time {
	last := t.Created
	for _, c := range []time.Time{t.RunStart, t.Finished} {
		if c.After(last) {
			last = c
		}
	}
	if a, err := time.ParseInLocation("2006-01-02 15:04:05", t.Last.TS, time.Local); err == nil && a.After(last) {
		last = a
	}
	return last
}

// viewAgents (GET /agents) lists the configured agents and what each one does.
func (s *webServer) viewAgents(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	list, lerr := s.agents.get(s.cfg.agentCmd)
	var rows []agentRow
	for _, a := range list {
		row := agentRow{agentEntry: a, State: "free", StateClass: "st-dim"}
		best := 99
		for _, t := range snap.All {
			if t.Agent != a.Name {
				continue
			}
			if la := lastActivity(t); la.After(row.LastAt) {
				row.LastAt = la
			}
			if !t.Active() {
				continue
			}
			row.Tasks = append(row.Tasks, makeRow(snap, t))
			if p := statusPrio(t); p < best {
				best, row.State, row.StateClass = p, t.Status, statusClass(t.Status)
			}
			if t.Status == "paused" {
				m := readMeta(filepath.Join(t.Dir, "meta.env"))
				row.Paused = pausedText(m)
			}
		}
		sort.Slice(row.Tasks, func(i, j int) bool { return row.Tasks[i].ID < row.Tasks[j].ID })
		if p := snap.Agents[a.Name]; p != nil {
			row.Presence = p.Status
		}
		if !row.LastAt.IsZero() {
			row.Last = row.LastAt.Format("2006-01-02 15:04")
		}
		rows = append(rows, row)
	}
	snapJobs, jerr := readWebJobs(s.cfg.busDir)
	if jerr != nil {
		viewError(s, w, r, jerr)
		return
	}
	if snapJobs != nil {
		now := time.Now()
		for nodeName, node := range snapJobs.Nodes {
			nodeOffline := webNodeOffline(node, now, s.cfg.nodeTTL)
			agents := node.Heartbeat.Agents
			if len(agents) == 0 && len(node.Heartbeat.FreeAgents) > 0 {
				for _, fa := range node.Heartbeat.FreeAgents {
					agents = append(agents, webNodeAgent{Name: fa.Name, Tier: string(fa.Tier)})
				}
			}
			for _, na := range agents {
				fullName := na.Name + "@" + nodeName
				isFree := false
				for _, fa := range node.Heartbeat.FreeAgents {
					if fa.Name == na.Name {
						isFree = true
						break
					}
				}
				defState := "unavailable"
				if nodeOffline {
					defState = "offline"
				} else if isFree {
					defState = "free"
				}
				row := agentRow{
					agentEntry: agentEntry{
						Name:  fullName,
						Type:  na.Type,
						Model: na.Model,
						Tier:  na.Tier,
					},
					State:      defState,
					StateClass: statusClass(defState),
				}
				for _, j := range snapJobs.Jobs {
					if j.Node != nodeName {
						continue
					}
					a := snapJobs.activityOf(j)
					if jobAgent(a, j) != na.Name {
						continue
					}
					tFin := jobFinishedAt(j)
					if tFin.After(row.LastAt) {
						row.LastAt = tFin
					}
					for _, tr := range j.History {
						if tr.At.After(row.LastAt) {
							row.LastAt = tr.At
						}
					}
					if j.State != "done" && j.State != "failed" && j.State != "stopped" {
						row.State = j.State
						row.StateClass = statusClass(j.State)
						if !node.LastSeen.IsZero() && node.LastSeen.After(row.LastAt) {
							row.LastAt = node.LastSeen
						}
						link := "/jobs/" + url.PathEscape(j.Request.ID)
						if j.TaskID != "" && snap.All[j.TaskID+"@"+nodeName] != nil {
							link = "/task/" + url.PathEscape(j.TaskID+"@"+nodeName)
						} else {
							for _, t := range snap.All {
								if t.JobID != "" && t.JobID == j.Request.ID {
									link = "/task/" + url.PathEscape(t.ID)
									break
								}
							}
						}
						jRow := taskRow{
							ID:          j.Request.ID,
							Name:        fullName,
							Status:      j.State,
							StatusLabel: j.State,
							StatusClass: statusClass(j.State),
							Repo:        j.Request.Repository,
							Link:        link,
							Running:     j.State == "running",
						}
						if n := len(a.Lines); n > 0 {
							jRow.Last = a.Lines[n-1]
						} else if a.Stage != "" {
							jRow.Last = a.Stage
						}
						row.Tasks = append(row.Tasks, jRow)
					}
				}
				if !row.LastAt.IsZero() {
					row.Last = row.LastAt.Format("2006-01-02 15:04")
				}
				rows = append(rows, row)
			}
		}
	}
	// working agents first, then by their latest activity, the never active last
	sort.SliceStable(rows, func(i, j int) bool {
		wi, wj := len(rows[i].Tasks) > 0, len(rows[j].Tasks) > 0
		if wi != wj {
			return wi
		}
		if !rows[i].LastAt.Equal(rows[j].LastAt) {
			return rows[i].LastAt.After(rows[j].LastAt)
		}
		return rows[i].Name < rows[j].Name
	})
	s.render(w, r, http.StatusOK, "agents", "Agents", snap, map[string]any{"Rows": rows, "Error": lerr, "Queue": snap.QueueLine(s.cfg.queueMax)})
}

// pausedText describes a pause from meta.env (WAKE_AT, PAUSE_REASON).
func pausedText(m map[string]string) string {
	out := ""
	if at, err := strconv.ParseInt(m["WAKE_AT"], 10, 64); err == nil && at > 0 {
		out = "until " + time.Unix(at, 0).Format("15:04")
	}
	switch m["PAUSE_REASON"] {
	case "quota":
		out += " (quota exhausted)"
	case "rate_limit":
		out += " (rate-limited by the provider)"
	case "overloaded":
		out += " (model overloaded at the provider)"
	case "wake_loop":
		out += " (wake-loop guard)"
	}
	return strings.TrimSpace(out)
}

// ---------------------------------------------------------------- task

// taskTabs are the sections of the task view, in menu order.
var taskTabs = []struct{ Key, Label string }{
	{"activity", "Activity"}, {"prompt", "Prompt"}, {"questions", "Questions"}, {"investigations", "Investigations"},
	{"report", "Final message"}, {"errors", "Errors"}, {"notebook", "Notebook"}, {"meta", "Details"},
}

// actRow is one activity entry.
type actRow struct {
	TS   string // HH:MM:SS
	Icon string // the panel's icon of the kind
	Kind string // text | cmd | edit | tool | mcp | err | info
	Text string // the step
}

// kv is one key/value line.
type kv struct {
	Key, Value string // the key, the value
}

// fileText is a named text file.
type fileText struct {
	Name, Text string // the file name, its content
	Current    bool   // the latest one (prompt of the current run, questions waiting for answers)
}

// taskData is the task view.
type taskData struct {
	T         *Task      // the task
	Row       taskRow    // its row (status, time, stage)
	Tab       string     // the shown section
	Tabs      []kv       // the sections (Key = key, Value = label)
	Paused    string     // pause description (paused tasks)
	Acts      []actRow   // activity (tab activity)
	ActOffset int64      // size of activity.jsonl when rendered (the stream continues from there)
	Files     []fileText // prompts, question rounds or error logs (other tabs)
	Invs      []invRow   // investigations of the task (tab investigations)
	Text      string     // the final message (tab report)
	Notes     notesView  // the repository notebook (tab notebook)
	Meta      []kv       // meta.env (tab details)
}

// findTask returns a task of the snapshot by its full ID or a unique prefix. It compares instead of indexing: the
// request only selects one of the tasks that exist, it never becomes part of a path.
func findTask(snap *Snapshot, id string) *Task {
	var hit *Task
	for k, t := range snap.All {
		switch {
		case k == id:
			return t
		case strings.HasPrefix(k, id):
			if hit != nil {
				return nil // ambiguous prefix
			}
			hit = t
		}
	}
	return hit
}

// viewTask (GET /task/{id}?tab=…) shows one task.
func (s *webServer) viewTask(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	t := findTask(snap, r.PathValue("id"))
	if t == nil {
		viewError(s, w, r, webErr(ViewNotFound, nil, "no task %q", r.PathValue("id")))
		return
	}
	tab := r.URL.Query().Get("tab")
	if tab == "" {
		tab = "activity"
	}
	d := taskData{T: t, Row: makeRow(snap, t), Tab: tab}
	for _, tb := range taskTabs {
		d.Tabs = append(d.Tabs, kv{tb.Key, tb.Label})
	}
	if t.Status == "paused" {
		d.Paused = pausedText(readMeta(filepath.Join(t.Dir, "meta.env")))
	}
	switch tab {
	case "activity":
		for _, a := range readActivity(t.Dir, 500) {
			d.Acts = append(d.Acts, actRow{TS: shortTS(a.TS), Icon: actIcon(a.Kind), Kind: a.Kind, Text: a.Text})
		}
		if st, err := os.Stat(filepath.Join(t.Dir, "activity.jsonl")); err == nil {
			d.ActOffset = st.Size()
		}
	case "prompt":
		for n := t.Run; n >= 1; n-- {
			d.Files = append(d.Files, fileText{Name: fmt.Sprintf("prompt-%d.md (run %d)", n, n),
				Text: readFile(filepath.Join(t.Dir, fmt.Sprintf("prompt-%d.md", n))), Current: n == t.Run})
		}
	case "questions":
		if t.Node != "" {
			matches, gerr := filepath.Glob(filepath.Join(t.Dir, "questions-*.md"))
			if gerr != nil {
				viewError(s, w, r, webErr(ViewStorage, gerr, "cannot list questions of %s", t.ID))
				return
			}
			sort.Sort(sort.Reverse(sort.StringSlice(matches)))
			for i, p := range matches {
				base := filepath.Base(p)
				var round int
				if n, err := fmt.Sscanf(base, "questions-%d.md", &round); err == nil && n == 1 {
					d.Files = append(d.Files, fileText{
						Name:    fmt.Sprintf("questions-%d.md (round %d)", round, round),
						Text:    readFile(p),
						Current: i == 0 && t.Status == "waiting",
					})
				}
			}
		} else {
			for n := t.Round; n >= 1; n-- {
				d.Files = append(d.Files, fileText{Name: fmt.Sprintf("questions-%d.md (round %d)", n, n),
					Text: readFile(filepath.Join(t.Dir, fmt.Sprintf("questions-%d.md", n))), Current: n == t.Round && t.Status == "waiting"})
			}
		}
	case "investigations":
		for _, inv := range snap.Invs {
			if inv.FromTask == t.ID || (t.Kind == "investigation" && fmt.Sprint(inv.ID) == t.InvID) {
				d.Invs = append(d.Invs, makeInvRow(snap, inv))
			}
		}
	case "report":
		d.Text = readFile(filepath.Join(t.Dir, "final.md"))
	case "errors":
		logs, gerr := filepath.Glob(filepath.Join(t.Dir, "err-*.log"))
		if gerr != nil {
			viewError(s, w, r, webErr(ViewStorage, gerr, "cannot list the error logs of %s", t.ID))
			return
		}
		sort.Sort(sort.Reverse(sort.StringSlice(logs)))
		for _, p := range logs {
			if txt := strings.Join(tailLines(p, 200, 256*1024), "\n"); strings.TrimSpace(txt) != "" {
				d.Files = append(d.Files, fileText{Name: filepath.Base(p), Text: txt})
			}
		}
	case "notebook":
		d.Notes = notebook(s.cfg.busDir, t.Repo)
	case "meta":
		var m map[string]string
		if t.Node != "" {
			m = readMetadataJSON(filepath.Join(t.Dir, "metadata.json"))
		} else {
			m = readMeta(filepath.Join(t.Dir, "meta.env"))
		}
		for k, v := range m {
			d.Meta = append(d.Meta, kv{k, v})
		}
		sort.Slice(d.Meta, func(i, j int) bool { return d.Meta[i].Key < d.Meta[j].Key })
	default:
		viewError(s, w, r, webErr(ViewBadParameter, nil, "no section %q", tab))
		return
	}
	s.render(w, r, http.StatusOK, "task", t.Label()+" · "+t.ID, snap, d)
}

// streamTask (GET /events/task/{id}?offset=N) sends new activity entries and status changes of a task.
func (s *webServer) streamTask(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	t := findTask(snap, r.PathValue("id"))
	if t == nil {
		http.Error(w, "no such task", http.StatusNotFound)
		return
	}
	off, err := strconv.ParseInt(r.URL.Query().Get("offset"), 10, 64)
	if err != nil || off < 0 {
		off = 0
	}
	f, ok := sseStream(w)
	if !ok {
		return
	}
	path := filepath.Join(t.Dir, "activity.jsonl")
	lastHead := s.fragment("task-head", makeRow(snap, t))
	tk := time.NewTicker(s.cfg.poll)
	defer tk.Stop()
	for {
		select {
		case <-r.Context().Done():
			return
		case <-tk.C:
		}
		var acts []Activity
		acts, off = readActivitySince(path, off)
		for _, a := range acts {
			if !sseSend(w, f, "act", s.fragment("act-item", actRow{TS: shortTS(a.TS), Icon: actIcon(a.Kind), Kind: a.Kind, Text: a.Text})) {
				return
			}
		}
		cur := s.snapshot()
		if ct := cur.All[t.ID]; ct != nil {
			if head := s.fragment("task-head", makeRow(cur, ct)); head != lastHead {
				if !sseSend(w, f, "head", head) {
					return
				}
				lastHead = head
			}
		}
	}
}

// readActivitySince reads the complete activity lines appended after off; it returns them and the new offset (a
// truncated file starts again from the beginning; an incomplete last line is read next time).
func readActivitySince(path string, off int64) ([]Activity, int64) {
	f, err := os.Open(path) // #nosec G304 -- the path is a task or bus file the program lists itself; no request text reaches it
	if err != nil {
		return nil, off // not created yet, or unreadable (the panel logs that)
	}
	defer closeQuietly(f, path)
	st, err := f.Stat()
	if err != nil {
		return nil, off
	}
	if st.Size() < off {
		off = 0
	}
	if _, err := f.Seek(off, io.SeekStart); err != nil {
		return nil, off
	}
	var out []Activity
	br := bufio.NewReader(f)
	for {
		line, err := br.ReadBytes('\n')
		if err != nil {
			break
		}
		off += int64(len(line))
		var a Activity
		if json.Unmarshal(line, &a) == nil {
			out = append(out, a)
		}
	}
	return out, off
}

// ---------------------------------------------------------------- notebook

// notesView is a repository notebook.
type notesView struct {
	Repo     string      // repository
	Sections []kv        // sections (Key = name, Value = text)
	Entries  []noteEntry // the latest log entries, newest first
}

// notebook reads a repository notebook.
func notebook(busDir, repo string) notesView {
	n := notesView{Repo: repo}
	if repo == "" {
		return n
	}
	secs, entries := Notebook(busDir, repo, 100)
	for name, sec := range secs {
		n.Sections = append(n.Sections, kv{name, sec.Text})
	}
	sort.Slice(n.Sections, func(i, j int) bool { return n.Sections[i].Key < n.Sections[j].Key })
	for i := len(entries) - 1; i >= 0; i-- {
		n.Entries = append(n.Entries, entries[i])
	}
	return n
}

// ---------------------------------------------------------------- teams

// teamRow is a team in the list.
type teamRow struct {
	ID, Lead, States string // team ID, coordinator, member states ("2 active, 1 sleeping")
	Done             bool   // finished
	Members          int    // number of members
	Todo, Claimed    int    // board items to do / in progress
	DoneItems        int    // board items done
}

// viewTeams (GET /teams) lists the teams, unfinished first.
func (s *webServer) viewTeams(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	var rows []teamRow
	for id, t := range snap.Teams {
		row := teamRow{ID: id, Lead: t.Lead, Done: t.Done, Members: len(t.Members)}
		states := map[string]int{}
		for _, m := range t.Members {
			states[m.State]++
		}
		var parts []string
		for st, n := range states {
			parts = append(parts, fmt.Sprintf("%d %s", n, st))
		}
		sort.Strings(parts)
		row.States = strings.Join(parts, ", ")
		for _, it := range t.Board {
			switch it.Status {
			case "todo":
				row.Todo++
			case "claimed":
				row.Claimed++
			default:
				row.DoneItems++
			}
		}
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool {
		if rows[i].Done != rows[j].Done {
			return !rows[i].Done
		}
		return rows[i].ID > rows[j].ID
	})
	total := len(rows)
	if r.URL.Query().Get("all") != "1" && total > teamsShown {
		rows = rows[:teamsShown]
	}
	s.render(w, r, http.StatusOK, "teams", "Teams", snap, map[string]any{"Rows": rows, "Total": total, "Cut": len(rows) < total})
}

// teamsShown is how many teams /teams lists before "all".
const teamsShown = 20

// memberRow is a team member.
type memberRow struct {
	Name, State, StateClass, Tier, Task string   // name, state, its CSS class, tier, task ID
	Lead                                bool     // the coordinator
	Row                                 *taskRow // its task's row (nil when the task directory is gone)
}

// consultRow is a consultation with every asked member's answer.
type consultRow struct {
	ConsultInfo      // the consultation
	Answers     []kv // member → answer ("NO ANSWER" after the limit)
}

// teamData is the team view.
type teamData struct {
	ID       string       // team ID
	T        *TeamInfo    // the team
	Members  []memberRow  // its members
	Board    []*BoardItem // its board, by number
	Consults []consultRow // its consultations, newest first
	Msgs     []msgRow     // its messages (the latest)
	Notes    notesView    // the notebook of its repository
}

// viewTeam (GET /team/{id}) shows a team: members, board, consultations, quiz, messages, notebook.
func (s *webServer) viewTeam(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	id := r.PathValue("id")
	t := snap.Teams[id]
	if t == nil {
		viewError(s, w, r, webErr(ViewNotFound, nil, "no team %q", id))
		return
	}
	d := teamData{ID: id, T: t}
	members := map[string]bool{}
	repo := ""
	for name, m := range t.Members {
		members[name] = true
		mr := memberRow{Name: name, State: m.State, StateClass: statusClass(m.State), Tier: m.Tier, Task: m.Task, Lead: name == t.Lead}
		if tk := snap.All[m.Task]; tk != nil {
			row := makeRow(snap, tk)
			mr.Row = &row
			if repo == "" || mr.Lead {
				repo = tk.Repo
			}
		}
		d.Members = append(d.Members, mr)
	}
	sort.Slice(d.Members, func(i, j int) bool {
		if d.Members[i].Lead != d.Members[j].Lead {
			return d.Members[i].Lead
		}
		return d.Members[i].Name < d.Members[j].Name
	})
	for _, it := range t.Board {
		d.Board = append(d.Board, it)
	}
	sort.Slice(d.Board, func(i, j int) bool { return itemNum(d.Board[i].ID) > itemNum(d.Board[j].ID) }) // newest first
	for _, c := range t.Consults {
		cr := consultRow{ConsultInfo: *c}
		for _, n := range c.Asked {
			ans := "no answer yet"
			if rep := c.Replies[n]; rep != nil {
				ans = rep.Text
				if rep.Late {
					ans += " (LATE)"
				}
			} else if dl, err := time.ParseInLocation("2006-01-02 15:04:05", c.Deadline, time.Local); err == nil && time.Now().After(dl) {
				ans = "NO ANSWER (the time limit passed)"
			}
			cr.Answers = append(cr.Answers, kv{n, ans})
		}
		d.Consults = append(d.Consults, cr)
	}
	sort.Slice(d.Consults, func(i, j int) bool { return itemNum(d.Consults[i].ID) > itemNum(d.Consults[j].ID) })
	for _, m := range readMessages(s.cfg.busDir, 2000) {
		if m.To == "team:"+id || (members[m.From] && (members[m.To] || m.To == "all")) || (m.From == "agentbus" && members[m.To]) {
			d.Msgs = append(d.Msgs, makeMsgRow(m))
		}
	}
	if len(d.Msgs) > 200 {
		d.Msgs = d.Msgs[len(d.Msgs)-200:]
	}
	d.Notes = notebook(s.cfg.busDir, repo)
	s.render(w, r, http.StatusOK, "team", "Team "+strings.TrimSuffix(id, "-team"), snap, d)
}

// itemNum is the number in an ID like "B12" or "C3" (0 when there is none).
func itemNum(id string) int {
	n, err := strconv.Atoi(strings.TrimLeft(id, "BCH#"))
	if err != nil {
		return 0 // a malformed ID sorts first
	}
	return n
}

// ---------------------------------------------------------------- bus

// msgRow is one bus message.
type msgRow struct {
	ID                 int64  // message ID
	TS, From, To, Kind string // time, sender, recipient, kind
	Text               string // body
	Class              string // CSS class (by kind)
	Pair               string // CSS class of the direction "from → to" (one of pairColours)
}

// pairColours is the number of colours a direction of messages can get (classes pair-0 … in app.css).
const pairColours = 12

// pairClass picks the colour class of a direction: the same "from → to" always gets the same one, the reverse
// direction usually another.
func pairClass(from, to string) string {
	h := uint32(2166136261) // FNV-1a, computed in place: nothing here can fail
	for _, c := range []byte(from + "\x00" + to) {
		h ^= uint32(c)
		h *= 16777619
	}
	return fmt.Sprintf("pair-%d", h%pairColours)
}

// makeMsgRow builds a message row.
func makeMsgRow(m Message) msgRow {
	k := m.Kind
	if k == "" {
		k = "chat"
	}
	return msgRow{ID: m.ID, TS: shortTS(m.TS), From: m.From, To: m.To, Kind: k, Text: m.Text, Class: "mk-" + k, Pair: pairClass(m.From, m.To)}
}

// readMessages reads the last n bus messages.
func readMessages(busDir string, n int) []Message {
	var out []Message
	for _, l := range tailLines(filepath.Join(busDir, "messages.jsonl"), n, 8*1024*1024) {
		var m Message
		if json.Unmarshal([]byte(l), &m) == nil {
			out = append(out, m)
		}
	}
	return out
}

// busFilter selects bus messages (the query of /bus and /events/bus).
type busFilter struct {
	Team, Agent, Kind, Q string // team ID, agent (sender or recipient), kind, text to find
}

// match reports whether a message passes the filter.
func (f busFilter) match(m Message, teams map[string]*TeamInfo) bool {
	if f.Kind != "" && m.Kind != f.Kind && (f.Kind != "chat" || m.Kind != "") { // "" is an old chat message
		return false
	}
	if f.Agent != "" && m.From != f.Agent && m.To != f.Agent {
		return false
	}
	if f.Team != "" {
		t := teams[f.Team]
		if m.To != "team:"+f.Team && (t == nil || (t.Members[m.From] == nil && t.Members[m.To] == nil)) {
			return false
		}
	}
	return f.Q == "" || strings.Contains(strings.ToLower(m.Text), strings.ToLower(f.Q))
}

// filterFrom reads the filter from a request.
func filterFrom(r *http.Request) busFilter {
	q := r.URL.Query()
	return busFilter{Team: q.Get("team"), Agent: q.Get("agent"), Kind: q.Get("kind"), Q: q.Get("q")}
}

// viewBus (GET /bus?team=&agent=&kind=&q=) shows the bus messages, live.
func (s *webServer) viewBus(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	f := filterFrom(r)
	var rows []msgRow
	for _, m := range readMessages(s.cfg.busDir, 3000) {
		if f.match(m, snap.Teams) {
			rows = append(rows, makeMsgRow(m))
		}
	}
	if len(rows) > 300 {
		rows = rows[len(rows)-300:]
	}
	for i, j := 0, len(rows)-1; i < j; i, j = i+1, j-1 {
		rows[i], rows[j] = rows[j], rows[i] // newest first
	}
	var teams, agents []string
	for id := range snap.Teams {
		teams = append(teams, id)
	}
	seen := map[string]bool{}
	for _, t := range snap.All {
		if !seen[t.Agent] {
			seen[t.Agent] = true
			agents = append(agents, t.Agent)
		}
	}
	sort.Sort(sort.Reverse(sort.StringSlice(teams)))
	sort.Strings(agents)
	off := int64(0)
	if st, err := os.Stat(filepath.Join(s.cfg.busDir, "messages.jsonl")); err == nil {
		off = st.Size()
	}
	s.render(w, r, http.StatusOK, "bus", "Bus", snap, map[string]any{"Rows": rows, "F": f, "Teams": teams, "Agents": agents,
		"Kinds": []string{"chat", "checkpoint", "board_item_added", "board_result", "script", "consult_request", "consult_reply",
			"investigation_finished", "review_queued", "team_finished"}, "Offset": off, "Query": r.URL.RawQuery})
}

// streamBus (GET /events/bus?offset=N&…filter) sends new messages that pass the filter.
func (s *webServer) streamBus(w http.ResponseWriter, r *http.Request) {
	filter := filterFrom(r)
	off, err := strconv.ParseInt(r.URL.Query().Get("offset"), 10, 64)
	if err != nil || off < 0 {
		off = 0
	}
	f, ok := sseStream(w)
	if !ok {
		return
	}
	path := filepath.Join(s.cfg.busDir, "messages.jsonl")
	tk := time.NewTicker(s.cfg.poll)
	defer tk.Stop()
	for {
		select {
		case <-r.Context().Done():
			return
		case <-tk.C:
		}
		var lines []string
		lines, off = readLinesSince(path, off)
		if len(lines) == 0 {
			continue
		}
		teams := s.snapshot().Teams
		for _, l := range lines {
			var m Message
			if json.Unmarshal([]byte(l), &m) != nil || !filter.match(m, teams) {
				continue
			}
			if !sseSend(w, f, "msg", s.fragment("msg-item", makeMsgRow(m))) {
				return
			}
		}
	}
}

// readLinesSince reads the complete lines appended to a file after off; it returns them and the new offset.
func readLinesSince(path string, off int64) ([]string, int64) {
	f, err := os.Open(path) // #nosec G304 -- the path is a task or bus file the program lists itself; no request text reaches it
	if err != nil {
		return nil, off
	}
	defer closeQuietly(f, path)
	st, err := f.Stat()
	if err != nil {
		return nil, off
	}
	if st.Size() < off {
		off = 0
	}
	if _, err := f.Seek(off, io.SeekStart); err != nil {
		return nil, off
	}
	var out []string
	br := bufio.NewReader(f)
	for {
		line, err := br.ReadString('\n')
		if err != nil {
			break
		}
		off += int64(len(line))
		out = append(out, strings.TrimRight(line, "\n"))
	}
	return out, off
}

// ---------------------------------------------------------------- questions

// waitingRow is a task waiting for answers.
type waitingRow struct {
	Row   taskRow // the task
	Round int     // the question round
	Count int     // questions in the file
}

// viewQuestions (GET /questions) lists every task waiting for answers.
func (s *webServer) viewQuestions(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	var rows []waitingRow
	for _, t := range snap.Waiting() {
		doc := parseQuestions(readFile(filepath.Join(t.Dir, fmt.Sprintf("questions-%d.md", t.Round))))
		rows = append(rows, waitingRow{Row: makeRow(snap, t), Round: t.Round, Count: len(doc.Questions)})
	}
	s.render(w, r, http.StatusOK, "questions", "Questions", snap, rows)
}

// answerQ is one question of the answer form.
type answerQ struct {
	Q      *question // the question
	Index  int       // its position (form field names: c<Index>, t<Index>)
	Choice string    // the selected choice: "ok", an option letter or "own"
	Text   string    // the text field
	Err    string    // why it was not accepted
}

// answerData is the answer form.
type answerData struct {
	Row     taskRow   // the task
	ID      string    // task ID
	Round   int       // question round
	File    string    // the questions file
	Intro   string    // the lines above the first question (title, instructions)
	Qs      []answerQ // the questions
	Message string    // a notice (draft saved, answers sent, a failure)
	Sent    bool      // the answers were sent and the task resumed
	Failed  bool      // the message is a failure
}

// loadAnswer reads the questions file of a waiting task.
func (s *webServer) loadAnswer(snap *Snapshot, id string) (*Task, string, *questionsDoc, *OpError[AnswerCause]) {
	t := findTask(snap, id)
	if t == nil {
		return nil, "", nil, webErr(AnswerNoQuestions, nil, "no task %q", id)
	}
	if t.Node != "" || t.Status != "waiting" || t.Round < 1 {
		return t, "", nil, webErr(AnswerNotWaiting, nil, "%s is %s — it does not wait for answers", t.ID, t.Status)
	}
	path := filepath.Join(t.Dir, fmt.Sprintf("questions-%d.md", t.Round))
	data, err := os.ReadFile(path) // #nosec G304 -- the path is the questions file of a listed task (t.Dir + round); see insideDir
	if err != nil {
		return t, path, nil, webErr(AnswerNoQuestions, err, "cannot read %s", path)
	}
	doc := parseQuestions(string(data))
	if len(doc.Questions) == 0 {
		return t, path, nil, webErr(AnswerNoQuestions, nil, "%s has no questions in the \"## N.\" format", path)
	}
	return t, path, doc, nil
}

// introOf returns the lines of a questions file above its first question, without the answering instructions.
func introOf(text string) string {
	var out []string
	for _, l := range strings.Split(text, "\n") {
		if qHeaderRe.MatchString(l) {
			break
		}
		if !strings.HasPrefix(l, "> Answer every question after ") {
			out = append(out, l)
		}
	}
	return strings.TrimSpace(strings.Join(out, "\n"))
}

// viewAnswer (GET /questions/{id}) shows the answer form of a waiting task.
func (s *webServer) viewAnswer(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	t, path, doc, aerr := s.loadAnswer(snap, r.PathValue("id"))
	if aerr != nil {
		viewError(s, w, r, aerr)
		return
	}
	d := answerData{Row: makeRow(snap, t), ID: t.ID, Round: t.Round, File: path, Intro: introOf(strings.Join(doc.lines, "\n"))}
	for i, q := range doc.Questions {
		c, txt := splitVerdict(q)
		d.Qs = append(d.Qs, answerQ{Q: q, Index: i, Choice: c, Text: txt})
	}
	s.render(w, r, http.StatusOK, "answer", "Answer · "+t.Label(), snap, d)
}

// postAnswer (POST /questions/{id}, action=draft|send) saves the verdicts into the questions file; "send" also
// marks it ANSWERED and calls `agent _answers_saved` — the same path as saving the file in an editor.
func (s *webServer) postAnswer(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	t, path, doc, aerr := s.loadAnswer(snap, r.PathValue("id"))
	if aerr != nil {
		viewError(s, w, r, aerr)
		return
	}
	if err := r.ParseForm(); err != nil {
		viewError(s, w, r, webErr(ViewBadParameter, err, "the form"))
		return
	}
	send := r.PostFormValue("action") == "send"
	d := answerData{Row: makeRow(snap, t), ID: t.ID, Round: t.Round, File: path, Intro: introOf(strings.Join(doc.lines, "\n"))}
	verdicts := map[int]string{}
	missing := 0
	for i, q := range doc.Questions {
		c, txt := r.PostFormValue(fmt.Sprintf("c%d", i)), r.PostFormValue(fmt.Sprintf("t%d", i))
		v := composeVerdict(q, c, txt)
		aq := answerQ{Q: q, Index: i, Choice: c, Text: txt}
		if v == "" && send {
			aq.Err = "This question has no verdict yet."
			missing++
		}
		verdicts[i] = v
		d.Qs = append(d.Qs, aq)
	}
	status := http.StatusOK
	switch {
	case missing > 0:
		e := webErr(AnswerIncomplete, nil, "%d question(s) without a verdict — nothing was sent", missing)
		logWarn(LogWebAnswersFailed, "task", t.ID, "cause", e.Cause.String(), "error", e.Message)
		d.Message, d.Failed, status = e.Message, true, e.Cause.HTTPStatus()
	default:
		if !insideDir(filepath.Join(s.cfg.home, "tasks"), path) {
			e := webErr(AnswerStorage, nil, "refusing to write %s: not inside the tasks directory", path)
			logWarn(LogWebAnswersFailed, "task", t.ID, "cause", e.Cause.String(), "error", e.Message)
			d.Message, d.Failed, status = e.Message, true, e.Cause.HTTPStatus()
			break
		}
		if werr := writeInPlace(path, doc.render(verdicts, send)); werr != nil {
			e := webErr(AnswerStorage, werr, "cannot write %s", path)
			logWarn(LogWebAnswersFailed, "task", t.ID, "cause", e.Cause.String(), "error", e.Message)
			d.Message, d.Failed, status = e.Message, true, e.Cause.HTTPStatus()
			break
		}
		logInfo(LogWebAnswersSaved, "task", t.ID, "round", t.Round, "sent", send)
		if !send {
			d.Message = "Draft saved (" + time.Now().Format("15:04:05") + "). Nothing was sent."
			break
		}
		out, cerr := callAgent(s.cfg.agentCmd, "_answers_saved", t.ID, strconv.Itoa(t.Round))
		if cerr != nil {
			e := webErr(AnswerResume, cerr, "the answers are saved, but sending them failed")
			logWarn(LogWebAnswersFailed, "task", t.ID, "cause", e.Cause.String(), "error", e.Message, "output", out)
			d.Message, d.Failed, status = e.Message, true, e.Cause.HTTPStatus()
			break
		}
		st := readMeta(filepath.Join(t.Dir, "meta.env"))["STATUS"]
		if st == "waiting" {
			d.Message, d.Failed = "The answers are saved and marked ANSWERED, but the task did not resume — see the notification or `agent log "+t.ID+"`.", true
			break
		}
		d.Message, d.Sent = "Answers sent — "+t.Label()+" resumes ("+st+").", true
	}
	s.render(w, r, status, "answer", "Answer · "+t.Label(), snap, d)
}

// writeInPlace rewrites an existing file without replacing it (same inode and mode: the path unit watching the
// file sees an ordinary modification).
func writeInPlace(path, text string) *StorageError {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_TRUNC, 0) // #nosec G703 G304 -- an existing questions file of a listed task, checked with insideDir
	if err != nil {
		return storageErr(err, "cannot open %s", path)
	}
	if _, err := io.WriteString(f, text); err != nil {
		if cerr := f.Close(); cerr != nil {
			return storageErr(cerr, "cannot close %s after a failed write", path)
		}
		return storageErr(err, "cannot write %s", path)
	}
	if err := f.Close(); err != nil {
		return storageErr(err, "cannot close %s", path)
	}
	return nil
}

// insideDir reports whether path lies inside dir (after cleaning both; symbolic links are not followed).
func insideDir(dir, path string) bool {
	rel, err := filepath.Rel(filepath.Clean(dir), filepath.Clean(path))
	return err == nil && rel != "." && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// ---------------------------------------------------------------- actions: stop a run, wake a team

// postStop (POST /task/{id}/stop) cuts a running agent's run short: `agent kick <task> web` (a team member sleeps
// and can be woken again; a solo task is stopped).
func (s *webServer) postStop(w http.ResponseWriter, r *http.Request) {
	t := findTask(s.snapshot(), r.PathValue("id"))
	if t == nil {
		http.Error(w, "no such task", http.StatusNotFound)
		return
	}
	if t.Node != "" {
		http.Error(w, "cannot stop mirrored task "+t.ID+" from the lead", http.StatusForbidden)
		return
	}
	out, cerr := callAgent(s.cfg.agentCmd, "kick", t.ID, "web")
	if cerr != nil {
		logWarn(LogWebActionFailed, "action", "kick", "task", t.ID, "cause", cerr.Cause.String(), "error", cerr.Message)
		http.Error(w, "cannot stop "+t.ID+": "+cerr.Message, http.StatusConflict)
		return
	}
	logInfo(LogWebAction, "action", "kick", "task", t.ID)
	writeText(w, strings.TrimSpace(out))
}

// postResume (POST /team/{id}/resume, mode=same|next, prompt) wakes a finished team from its last task:
// `agent run --resume <team> [--same-round] [prompt]` — the same round, or the next one after a merged PR.
func (s *webServer) postResume(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	id := r.PathValue("id")
	t := snap.Teams[id]
	if t == nil {
		viewError(s, w, r, webErr(ViewNotFound, nil, "no team %q", id))
		return
	}
	if !t.Done {
		viewError(s, w, r, webErr(ViewBadParameter, nil, "team %s is still working — nothing to wake", id))
		return
	}
	if err := r.ParseForm(); err != nil {
		viewError(s, w, r, webErr(ViewBadParameter, err, "the form"))
		return
	}
	args := []string{"run", "--resume", id}
	if r.PostFormValue("mode") == "same" {
		args = append(args, "--same-round")
	}
	if p := strings.TrimSpace(strings.ReplaceAll(r.PostFormValue("prompt"), "\r\n", "\n")); p != "" {
		args = append(args, p)
	}
	out, cerr := callAgent(s.cfg.agentCmd, args...)
	msg := map[string]any{"Team": id, "Output": strings.TrimSpace(out), "Failed": cerr != nil}
	if cerr != nil {
		logWarn(LogWebActionFailed, "action", "resume", "team", id, "cause", cerr.Cause.String(), "error", cerr.Message)
		msg["Output"] = cerr.Message
	} else {
		logInfo(LogWebAction, "action", "resume", "team", id, "mode", r.PostFormValue("mode"))
	}
	s.render(w, r, http.StatusOK, "resumed", "Wake team "+strings.TrimSuffix(id, "-team"), snap, msg)
}

// ---------------------------------------------------------------- investigations

// invRow is an investigation or a PR review.
type invRow struct {
	*Investigation        // the record
	StatusClass    string // CSS class of its status
	Review         bool   // a PR review
	Investigator   string // the investigator agent that worked on it ("" until one started)
	Model          string // that investigator's model
}

// makeInvRow builds an investigation row.
func makeInvRow(snap *Snapshot, inv *Investigation) invRow {
	cls := statusClass(inv.Status)
	switch inv.Status {
	case "queued", "waitroom":
		cls = "st-dim"
	case "waiting_human":
		cls = "st-waiting"
	}
	row := invRow{Investigation: inv, StatusClass: cls, Review: inv.Kind == "review"}
	if t := snap.All[inv.InvTask]; t != nil {
		row.Investigator, row.Model = t.Agent, t.Model
	}
	return row
}

// viewInvestigations (GET /investigations) lists investigations and reviews, newest first.
func (s *webServer) viewInvestigations(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	var rows []invRow
	for i := len(snap.Invs) - 1; i >= 0; i-- {
		rows = append(rows, makeInvRow(snap, snap.Invs[i]))
	}
	s.render(w, r, http.StatusOK, "investigations", "Investigations", snap, map[string]any{"Rows": rows, "Queue": snap.QueueLine(s.cfg.queueMax)})
}

// viewInvestigation (GET /investigation/{id}) shows the record of one investigation.
func (s *webServer) viewInvestigation(w http.ResponseWriter, r *http.Request) {
	snap := s.snapshot()
	id := strings.TrimPrefix(r.PathValue("id"), "#")
	for _, inv := range snap.Invs {
		if fmt.Sprint(inv.ID) == id {
			s.render(w, r, http.StatusOK, "investigation", fmt.Sprintf("Investigation #%d", inv.ID), snap,
				map[string]any{"I": makeInvRow(snap, inv), "Text": readFile(inv.File)})
			return
		}
	}
	viewError(s, w, r, webErr(ViewNotFound, nil, "no investigation #%s", id))
}

// ---------------------------------------------------------------- logs

// logSources are the logs the logs view shows (~/agents/logs/<name>.jsonl).
var logSources = []string{"agent", "agentbus", "agentlive"}

// logFile maps a log name from the request to its file name (a constant: the request never reaches the path).
func logFile(src string) (string, bool) {
	switch src {
	case "agent":
		return "agent.jsonl", true
	case "agentbus":
		return "agentbus.jsonl", true
	case "agentlive":
		return "agentlive.jsonl", true
	}
	return "", false
}

// logRow is one log record.
type logRow struct {
	TS        string    // time (local)
	Level     string    // log level
	Node      string    // node name; empty for local log entries
	Component string    // log component
	Event     string    // log event name
	Task      string    // task identifier
	TaskLink  string    // link to task on lead (/task/<id> or /task/<id>@<node>)
	Rest      string    // the other fields, "key=value …"
	rawTime   time.Time // timestamp used for chronological sorting
}

// logLevelRank orders levels for the minimum-level filter.
var logLevelRank = map[string]int{"debug": 0, "info": 1, "warn": 2, "error": 3}

// viewLogs (GET /logs?src=agent&level=warn&q=…&n=300) shows the latest records of a log.
func (s *webServer) viewLogs(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	src := q.Get("src")
	if src == "" {
		src = "agent"
	}
	file, known := logFile(src)
	if !known {
		viewError(s, w, r, webErr(ViewBadParameter, nil, "no log %q (agent, agentbus or agentlive)", src))
		return
	}
	n, err := strconv.Atoi(q.Get("n"))
	if err != nil || n <= 0 {
		n = 300
	}
	if n > 5000 {
		n = 5000
	}
	minLevel, find := q.Get("level"), strings.ToLower(q.Get("q"))
	nodeFilter := q.Get("node")

	var allNodes []string
	if remEntries, rerr := os.ReadDir(filepath.Join(s.cfg.busDir, "remote")); rerr == nil {
		for _, e := range remEntries {
			if e.IsDir() {
				allNodes = append(allNodes, e.Name())
			}
		}
		sort.Strings(allNodes)
	}

	type lineWithNode struct {
		line string
		node string
	}
	var lines []lineWithNode

	if nodeFilter == "" || nodeFilter == "all" || nodeFilter == "local" {
		for _, l := range tailLines(filepath.Join(s.cfg.home, "logs", file), n, 16*1024*1024) {
			lines = append(lines, lineWithNode{line: l, node: ""})
		}
	}

	if nodeFilter == "" || nodeFilter == "all" {
		for _, node := range allNodes {
			for _, l := range tailLines(filepath.Join(s.cfg.busDir, "remote", node, "logs.jsonl"), n, 16*1024*1024) {
				lines = append(lines, lineWithNode{line: l, node: node})
			}
		}
	} else if nodeFilter != "local" {
		for _, node := range allNodes {
			if node == nodeFilter {
				for _, l := range tailLines(filepath.Join(s.cfg.busDir, "remote", nodeFilter, "logs.jsonl"), n, 16*1024*1024) {
					lines = append(lines, lineWithNode{line: l, node: nodeFilter})
				}
				break
			}
		}
	}

	var rows []logRow
	for _, item := range lines {
		l := item.line
		if find != "" && !strings.Contains(strings.ToLower(l), find) {
			continue
		}
		var rec map[string]any
		if json.Unmarshal([]byte(l), &rec) != nil {
			continue
		}
		row := logRow{
			Level:     str(rec, "level"),
			Node:      item.node,
			Component: str(rec, "component"),
			Event:     str(rec, "event"),
			Task:      str(rec, "task"),
		}
		if minLevel != "" && logLevelRank[row.Level] < logLevelRank[minLevel] {
			continue
		}
		if row.Task != "" {
			if row.Node != "" {
				row.TaskLink = "/task/" + url.PathEscape(row.Task+"@"+row.Node)
			} else {
				row.TaskLink = "/task/" + url.PathEscape(row.Task)
			}
		}
		if ts, err := time.Parse(time.RFC3339, str(rec, "ts")); err == nil {
			row.rawTime = ts
			row.TS = ts.Local().Format("01-02 15:04:05")
		} else {
			row.TS = str(rec, "ts")
		}
		var rest []string
		keys := make([]string, 0, len(rec))
		for k := range rec {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			switch k {
			case "ts", "level", "component", "event", "task":
				continue
			}
			rest = append(rest, k+"="+fmt.Sprint(rec[k]))
		}
		row.Rest = strings.Join(rest, " ")
		rows = append(rows, row)
	}

	sort.SliceStable(rows, func(i, j int) bool {
		if !rows[i].rawTime.Equal(rows[j].rawTime) {
			return rows[i].rawTime.After(rows[j].rawTime)
		}
		return rows[i].TS > rows[j].TS
	})

	if len(rows) > n {
		rows = rows[:n]
	}

	s.render(w, r, http.StatusOK, "logs", "Logs", nil, map[string]any{
		"Rows": rows, "Src": src, "Sources": logSources, "Nodes": allNodes, "Node": nodeFilter,
		"Level": minLevel, "Q": q.Get("q"), "N": n, "Live": q.Get("live") == "1", "Query": r.URL.RawQuery,
	})
}

// str returns a field of a decoded JSON record as a string.
func str(rec map[string]any, k string) string {
	if v, ok := rec[k]; ok {
		return fmt.Sprint(v)
	}
	return ""
}

// webInstanceData is the data passed to the instance detail view.
type webInstanceData struct {
	Name       string         // node name
	Node       webJobNode     // node declaration
	URL        string         // node's own web interface URL
	NeverSeen  bool           // node was never seen
	Offline    bool           // node is currently offline
	Seen       string         // formatted last seen timestamp
	AllAgents  []webNodeAgent // all configured agents declared by the node
	FreeAgents []webJobAgent  // free agents declared by the node
	Jobs       []webJobRow    // jobs assigned to this node
}

// viewInstance (GET /instances/{node}) shows details of one registered node.
func (s *webServer) viewInstance(w http.ResponseWriter, r *http.Request) {
	snap, err := readWebJobs(s.cfg.busDir)
	if err != nil {
		viewError(s, w, r, err)
		return
	}
	name := r.PathValue("node")
	node, ok := snap.Nodes[name]
	if !ok {
		viewError(s, w, r, webErr(ViewNotFound, nil, "no node %q", name))
		return
	}
	now := time.Now()
	neverSeen := node.LastSeen.IsZero()
	offline := !neverSeen && webNodeOffline(node, now, s.cfg.nodeTTL)
	seen := ""
	if !neverSeen {
		seen = node.LastSeen.Format(time.RFC3339Nano)
	}
	allAgents := node.Heartbeat.Agents
	if len(allAgents) == 0 && len(node.Heartbeat.FreeAgents) > 0 {
		for _, fa := range node.Heartbeat.FreeAgents {
			allAgents = append(allAgents, webNodeAgent{Name: fa.Name, Tier: string(fa.Tier)})
		}
	}
	localSnap := s.snapshot()
	var assignedJobs []webJobRow
	for _, j := range snap.Jobs {
		if j.Node == name {
			assignedJobs = append(assignedJobs, buildWebJobRow(j, snap, localSnap, now, s.cfg.nodeTTL))
		}
	}
	data := webInstanceData{
		Name:       name,
		Node:       node,
		URL:        safeJobURL(node.Heartbeat.WebURL),
		NeverSeen:  neverSeen,
		Offline:    offline,
		Seen:       seen,
		AllAgents:  allAgents,
		FreeAgents: node.Heartbeat.FreeAgents,
		Jobs:       assignedJobs,
	}
	s.render(w, r, http.StatusOK, "instance", "Instance "+name, localSnap, data)
}

// webAgentData holds details of one local or remote agent for viewAgent.
type webAgentData struct {
	Name       string     // agent name, local or agent@node
	Remote     bool       // true for remote agent
	Node       string     // node name if remote
	Entry      agentEntry // agent configuration
	State      string     // free or task/job status
	StateClass string     // CSS class of status
	Tasks      []taskRow  // tasks or jobs
	Last       string     // formatted last activity
}

// viewAgent (GET /agents/{id}) shows details of one local or remote agent.
func (s *webServer) viewAgent(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	snap := s.snapshot()
	if strings.Contains(id, "@") {
		parts := strings.SplitN(id, "@", 2)
		agentName, nodeName := parts[0], parts[1]
		snapJobs, err := readWebJobs(s.cfg.busDir)
		if err != nil {
			viewError(s, w, r, err)
			return
		}
		node, ok := snapJobs.Nodes[nodeName]
		if !ok {
			viewError(s, w, r, webErr(ViewNotFound, nil, "no node %q for agent %q", nodeName, id))
			return
		}
		var targetAgent *webNodeAgent
		for i := range node.Heartbeat.Agents {
			if node.Heartbeat.Agents[i].Name == agentName {
				targetAgent = &node.Heartbeat.Agents[i]
				break
			}
		}
		if targetAgent == nil {
			for _, fa := range node.Heartbeat.FreeAgents {
				if fa.Name == agentName {
					targetAgent = &webNodeAgent{Name: fa.Name, Tier: string(fa.Tier)}
					break
				}
			}
		}
		if targetAgent == nil {
			viewError(s, w, r, webErr(ViewNotFound, nil, "no agent %q on node %q", agentName, nodeName))
			return
		}

		now := time.Now()
		nodeOffline := webNodeOffline(node, now, s.cfg.nodeTTL)
		isFree := false
		for _, fa := range node.Heartbeat.FreeAgents {
			if fa.Name == agentName {
				isFree = true
				break
			}
		}
		defState := "unavailable"
		if nodeOffline {
			defState = "offline"
		} else if isFree {
			defState = "free"
		}
		data := webAgentData{
			Name:   id,
			Remote: true,
			Node:   nodeName,
			Entry: agentEntry{
				Name:  id,
				Type:  targetAgent.Type,
				Model: targetAgent.Model,
				Tier:  targetAgent.Tier,
			},
			State:      defState,
			StateClass: statusClass(defState),
		}
		var lastAt time.Time
		for _, j := range snapJobs.Jobs {
			if j.Node != nodeName {
				continue
			}
			a := snapJobs.activityOf(j)
			if jobAgent(a, j) != agentName {
				continue
			}
			tFin := jobFinishedAt(j)
			if tFin.After(lastAt) {
				lastAt = tFin
			}
			for _, tr := range j.History {
				if tr.At.After(lastAt) {
					lastAt = tr.At
				}
			}
			if j.State != "done" && j.State != "failed" && j.State != "stopped" {
				data.State = j.State
				data.StateClass = statusClass(j.State)
				if !node.LastSeen.IsZero() && node.LastSeen.After(lastAt) {
					lastAt = node.LastSeen
				}
			}
			link := "/jobs/" + url.PathEscape(j.Request.ID)
			if j.TaskID != "" && snap.All[j.TaskID+"@"+nodeName] != nil {
				link = "/task/" + url.PathEscape(j.TaskID+"@"+nodeName)
			} else {
				for _, t := range snap.All {
					if t.JobID != "" && t.JobID == j.Request.ID {
						link = "/task/" + url.PathEscape(t.ID)
						break
					}
				}
			}
			jRow := taskRow{
				ID:          j.Request.ID,
				Name:        id,
				Status:      j.State,
				StatusLabel: j.State,
				StatusClass: statusClass(j.State),
				Repo:        j.Request.Repository,
				Link:        link,
				Running:     j.State == "running",
				FinishedAt:  tFin,
			}
			if n := len(a.Lines); n > 0 {
				jRow.Last = a.Lines[n-1]
			} else if a.Stage != "" {
				jRow.Last = a.Stage
			} else if j.Result != "" {
				jRow.Last = j.Result
			}
			data.Tasks = append(data.Tasks, jRow)
		}
		sort.SliceStable(data.Tasks, func(i, j int) bool {
			ti, tj := data.Tasks[i], data.Tasks[j]
			termI, termJ := jobTerminal(ti.Status), jobTerminal(tj.Status)
			if termI != termJ {
				return !termI
			}
			if !termI {
				if ti.Running != tj.Running {
					return ti.Running
				}
				return ti.ID < tj.ID
			}
			return ti.FinishedAt.After(tj.FinishedAt)
		})
		if !lastAt.IsZero() {
			data.Last = lastAt.Format("2006-01-02 15:04")
		}
		s.render(w, r, http.StatusOK, "agent", "Agent "+id, snap, data)
		return
	}

	// Local agent
	list, lerr := s.agents.get(s.cfg.agentCmd)
	if lerr != "" && len(list) == 0 {
		viewError(s, w, r, webErr(ViewStorage, nil, "cannot list agents: %s", lerr))
		return
	}
	var targetEntry *agentEntry
	for i := range list {
		if list[i].Name == id {
			targetEntry = &list[i]
			break
		}
	}
	if targetEntry == nil {
		viewError(s, w, r, webErr(ViewNotFound, nil, "no agent %q", id))
		return
	}

	data := webAgentData{
		Name:       id,
		Remote:     false,
		Entry:      *targetEntry,
		State:      "free",
		StateClass: "st-dim",
	}
	best := 99
	var lastAt time.Time
	for _, t := range snap.All {
		if t.Agent != id {
			continue
		}
		if la := lastActivity(t); la.After(lastAt) {
			lastAt = la
		}
		r := makeRow(snap, t)
		data.Tasks = append(data.Tasks, r)
		if t.Active() {
			if p := statusPrio(t); p < best {
				best, data.State, data.StateClass = p, t.Status, statusClass(t.Status)
			}
		}
	}
	sort.SliceStable(data.Tasks, func(i, j int) bool {
		ti, tj := snap.All[data.Tasks[i].ID], snap.All[data.Tasks[j].ID]
		if ti != nil && tj != nil {
			ai, aj := ti.Active(), tj.Active()
			if ai != aj {
				return ai
			}
			if ai {
				return statusPrio(ti) < statusPrio(tj)
			}
		}
		return data.Tasks[i].FinishedAt.After(data.Tasks[j].FinishedAt)
	})
	if !lastAt.IsZero() {
		data.Last = lastAt.Format("2006-01-02 15:04")
	}
	s.render(w, r, http.StatusOK, "agent", "Agent "+id, snap, data)
}
