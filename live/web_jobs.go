package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// NodeTTLCause identifies invalid heartbeat timeouts supplied to the web server.
type NodeTTLCause int

const (
	NodeTTLInvalid NodeTTLCause = iota // timeout is not a positive, representable duration in seconds
)

// String identifies the invalid timeout in startup diagnostics.
func (c NodeTTLCause) String() string {
	switch c {
	case NodeTTLInvalid:
		return "InvalidNodeTTL"
	}
	return fmt.Sprintf("NodeTTLCause(%d)", int(c))
}

// webNodeTTLFlag declares the optional heartbeat timeout without changing existing web invocations.
func webNodeTTLFlag(fs *flag.FlagSet) *string {
	return fs.String("node-ttl", "90", "node heartbeat timeout in seconds")
}

// parseWebNodeTTL uses the lead CLI's existing NODE_TTL_SEC syntax, including fractional seconds.
func parseWebNodeTTL(raw string) (time.Duration, *OpError[NodeTTLCause]) {
	ttl, err := time.ParseDuration(raw + "s")
	if err != nil || ttl <= 0 {
		return 0, webErr(NodeTTLInvalid, err, "node heartbeat timeout must be a positive duration in seconds")
	}
	return ttl, nil
}

// webJobsSnapshot reads only the public fields required by the two remote-job views.
// Atomic replacement of jobs.json gives each request a complete snapshot without a bus lock.
type webJobsSnapshot struct {
	Version int                   // persisted format discriminator; only version one is supported
	Jobs    []webJob              // jobs in submission order
	Nodes   map[string]webJobNode // node declarations by authenticated short name
}

// webJob is a read-only subset of a persisted lead job; unknown fields remain compatible.
type webJob struct {
	Request webJobRequest      // public request identity and requested agent selector
	State   string             // lifecycle label, displayed without executing state transitions
	Node    string             // assigned node, empty while queued
	TaskID  string             // task ID on the assigned node
	Agent   string             // actual chosen agent on the node; empty if unknown
	Branch  string             // branch pushed by the task
	PRURL   string             // optional pull request URL, sanitized before use as a link
	Result  string             // final summary or provisioning diagnostic
	History []webJobTransition // state transitions recorded by the lead
}

// webJobTransition records when a job entered a state.
type webJobTransition struct {
	State string    // lifecycle state
	At    time.Time // timestamp recorded by the lead
}

// webJobRequest holds public submission fields useful when scanning jobs.
type webJobRequest struct {
	ID            string   // submitter-chosen job ID
	Repository    string   // logical repository name
	Selector      string   // requested configured agent name or tier
	Requires      []string // required capabilities
	RepoURL       string   // canonical GitHub repository URL (https://github.com/<owner>/<repo>)
	RequesterTask string   // optional task ID of the requesting agent on the lead
	Team          string   // optional team on the lead for remote team member jobs
	Member        string   // optional remote member name in that team
}

// webJobNode is the read-only declaration and trusted heartbeat receipt time.
type webJobNode struct {
	Heartbeat webJobHeartbeat // last accepted node declaration
	LastSeen  time.Time       // precise lead receipt time in RFC3339Nano
}

// webNodeAgent identifies one configured agent declared by a node.
type webNodeAgent struct {
	Name  string // configured agent name
	Type  string // CLI type
	Model string // model name
	Tier  string // declared tier
}

// webJobHeartbeat contains the fields the lead may expose to a browser.
type webJobHeartbeat struct {
	Node         string           // authenticated node name
	Capabilities []string         // capabilities declared by the operator
	Agents       []webNodeAgent   // all configured agents declared by the node
	FreeAgents   []webJobAgent    // agents free at the last accepted heartbeat
	WebURL       string           // operator-supplied address of the node's web interface
	Activity     []webJobActivity // what the node's running jobs are doing (kept by the lead for its own jobs)
}

// webJobActivity is what one remote job is doing on its node.
type webJobActivity struct {
	ID    string   // the job
	Agent string   // the node's agent working on it
	Stage string   // the task's status on the node
	Lines []string // its latest activity lines, oldest first
}

// activityOf is the latest activity a node reported for a job (zero value: none).
func (s *webJobsSnapshot) activityOf(j webJob) webJobActivity {
	for _, a := range s.Nodes[j.Node].Heartbeat.Activity {
		if a.ID == j.Request.ID {
			return a
		}
	}
	return webJobActivity{}
}

// jobAgent names who works on a job: the persisted chosen agent, or the node's live activity agent if reported,
// or the requested selector when not a tier selector ("normal" or "worker") to avoid bogus agent links.
func jobAgent(a webJobActivity, j webJob) string {
	if j.Agent != "" {
		return j.Agent
	}
	if a.Agent != "" {
		return a.Agent
	}
	if j.Request.Selector == "normal" || j.Request.Selector == "worker" {
		return ""
	}
	return j.Request.Selector
}

// jobFinishedAt returns when a finished job ended, or zero if still active or unknown.
func jobFinishedAt(j webJob) time.Time {
	for i := len(j.History) - 1; i >= 0; i-- {
		h := j.History[i]
		if h.State == "done" || h.State == "failed" || h.State == "stopped" {
			if !h.At.IsZero() {
				return h.At
			}
		}
	}
	return time.Time{}
}

// gitHubBranchURL returns a tree link when repoURL is a canonical GitHub URL and branch is set.
func gitHubBranchURL(repoURL, branch string) string {
	if branch == "" || !strings.HasPrefix(repoURL, "https://github.com/") {
		return ""
	}
	u, err := url.Parse(repoURL)
	if err != nil || u.Scheme != "https" || u.Host != "github.com" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return ""
	}
	parts := strings.Split(strings.Trim(u.Path, "/"), "/")
	if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
		return ""
	}
	u.Path = "/" + parts[0] + "/" + parts[1] + "/tree/" + branch
	u.RawPath = ""
	return u.String()
}

// remoteRows are the dashboard rows of the lead's remote jobs: active jobs and jobs finished in the
// last 24 hours, newest first, linked to their page. Nil when this installation is not a lead.
func remoteRows(busDir string) []taskRow {
	snap, err := readWebJobs(busDir)
	if err != nil || snap == nil {
		return nil // no lead here, or no readable registry: the dashboard shows the local tasks only
	}
	var active, done []taskRow
	now := time.Now()
	for _, j := range snap.Jobs {
		if j.Node == "" {
			continue
		}
		a := snap.activityOf(j)
		agent := jobAgent(a, j)
		displayAgent := agent
		if displayAgent == "" {
			displayAgent = j.Request.Selector
		}
		r := taskRow{
			ID:          j.Request.ID,
			Name:        displayAgent + "@" + j.Node,
			Status:      j.State,
			StatusLabel: j.State,
			StatusClass: statusClass(j.State),
			Repo:        j.Request.Repository,
			Link:        "/jobs/" + url.PathEscape(j.Request.ID),
			Running:     false,
		}
		if j.State != "done" && j.State != "failed" && j.State != "stopped" {
			if n := len(a.Lines); n > 0 {
				r.Last = a.Lines[n-1]
			} else if a.Stage != "" {
				r.Last = a.Stage
			}
			active = append(active, r)
		} else {
			fin := jobFinishedAt(j)
			if fin.IsZero() || now.Sub(fin) >= dashRecent {
				continue
			}
			r.FinishedAt = fin
			r.Since = human(now.Sub(fin)) + " ago"
			r.Last = j.Result
			done = append(done, r)
		}
	}
	sort.SliceStable(done, func(i, j int) bool {
		if !done[i].FinishedAt.Equal(done[j].FinishedAt) {
			return done[i].FinishedAt.After(done[j].FinishedAt)
		}
		return done[i].ID > done[j].ID
	})
	return append(active, done...)
}

// viewJob (GET /jobs/{id}) is one remote job: its fields, its latest activity and a link to its task on the node.
func (s *webServer) viewJob(w http.ResponseWriter, r *http.Request) {
	snap, err := readWebJobs(s.cfg.busDir)
	if err != nil {
		viewError(s, w, r, err)
		return
	}
	id := r.PathValue("id")
	for _, j := range snap.Jobs {
		if j.Request.ID != id {
			continue
		}
		a := snap.activityOf(j)
		j.PRURL = safeJobURL(j.PRURL)
		row := buildWebJobRow(j, snap, s.snapshot(), time.Now(), s.cfg.nodeTTL)
		s.render(w, r, http.StatusOK, "job", "Job "+id, s.snapshot(), map[string]any{
			"J": row.webJob, "Agent": row.AgentName, "Activity": a, "NodeURL": row.NodeURL,
			"Offline": row.Offline, "NodeLink": row.NodeLink, "AgentLink": row.AgentLink,
			"BranchURL": row.BranchURL, "RequesterTask": row.RequesterTask,
			"RequesterTaskLink": row.RequesterTaskLink, "RequesterTeam": row.RequesterTeam,
			"RequesterTeamShort": row.RequesterTeamShort,
			"Team":               row.Team, "TeamShort": row.TeamShort,
			"ProxyTask": row.ProxyTask, "ProxyTaskLink": row.ProxyTaskLink})
		return
	}
	viewError(s, w, r, webErr(ViewNotFound, nil, "no job %q", id))
}

// webJobAgent identifies one configured free agent on the node.
type webJobAgent struct {
	Name string // configured local agent name
	Tier string // declared normal or worker tier, displayed as data
}

// readWebJobs distinguishes a missing registry from an unreadable or unsupported snapshot.
func readWebJobs(dir string) (*webJobsSnapshot, *OpError[ViewCause]) {
	data, err := os.ReadFile(filepath.Join(dir, "jobs.json")) // #nosec G304 -- dir is the configured local bus directory; no request or remote job field selects the path
	if errors.Is(err, os.ErrNotExist) {
		return &webJobsSnapshot{}, nil
	}
	if err != nil {
		return nil, webErr(ViewStorage, err, "cannot read jobs.json")
	}
	var snapshot webJobsSnapshot
	if err := json.Unmarshal(data, &snapshot); err != nil {
		return nil, webErr(ViewStorage, err, "cannot decode jobs.json")
	}
	if snapshot.Version != 1 {
		return nil, webErr(ViewStorage, nil, "unsupported jobs.json format %d", snapshot.Version)
	}
	return &snapshot, nil
}

// webJobRow adds safe browser links and heartbeat status to a public job record.
type webJobRow struct {
	webJob                    // persisted public fields
	NodeURL            string // safe node web/task URL, empty if unavailable
	Offline            bool   // assigned node is missing or has reached its heartbeat deadline
	NodeLink           string // /instances/<node> link
	AgentName          string // worker agent name or requested selector
	AgentLink          string // /agents/<agent>@<node> link
	BranchURL          string // canonical GitHub branch tree URL
	RequesterTask      string // requester task ID
	RequesterTaskLink  string // /task/<RequesterTask> link if task exists locally
	RequesterTeam      string // team ID if requester task is in a team
	RequesterTeamShort string // team ID without -team suffix
	Team               string // team ID for remote team member jobs
	TeamShort          string // team ID without -team suffix
	ProxyTask          string // proxy task ID on the lead for remote team member jobs
	ProxyTaskLink      string // /task/<ProxyTask> link if task exists locally
}

// buildWebJobRow constructs a webJobRow with links according to decision 7.
func buildWebJobRow(j webJob, snapshot *webJobsSnapshot, snapLocal *Snapshot, now time.Time, ttl time.Duration) webJobRow {
	node := snapshot.Nodes[j.Node]
	a := snapshot.activityOf(j)
	agent := jobAgent(a, j)
	displayAgent := agent
	if displayAgent == "" {
		displayAgent = j.Request.Selector
	}
	j.PRURL = safeJobURL(j.PRURL)
	row := webJobRow{
		webJob:        j,
		NodeURL:       jobNodeURL(node.Heartbeat.WebURL, j.TaskID),
		Offline:       j.Node != "" && webNodeOffline(node, now, ttl),
		AgentName:     displayAgent,
		BranchURL:     gitHubBranchURL(j.Request.RepoURL, j.Branch),
		RequesterTask: j.Request.RequesterTask,
	}
	if j.Node != "" {
		row.NodeLink = "/instances/" + url.PathEscape(j.Node)
		if agent != "" {
			row.AgentLink = "/agents/" + url.PathEscape(agent+"@"+j.Node)
		}
	}
	if row.RequesterTask != "" && snapLocal != nil {
		for _, t := range snapLocal.All {
			if t.ID == row.RequesterTask {
				row.RequesterTaskLink = "/task/" + url.PathEscape(t.ID)
				if t.Team != "" {
					row.RequesterTeam = t.Team
					row.RequesterTeamShort = strings.TrimSuffix(t.Team, "-team")
				}
				break
			}
		}
	}
	memberTeam := j.Request.Team
	var proxyTask string
	if snapLocal != nil {
		for _, t := range snapLocal.All {
			if t.JobID == j.Request.ID {
				proxyTask = t.ID
				if memberTeam == "" && t.Team != "" {
					memberTeam = t.Team
				}
				break
			}
		}
	}
	if proxyTask != "" {
		row.ProxyTask = proxyTask
		row.ProxyTaskLink = "/task/" + url.PathEscape(proxyTask)
	}
	if memberTeam != "" {
		row.Team = memberTeam
		row.TeamShort = strings.TrimSuffix(memberTeam, "-team")
	}
	return row
}

// webInstanceRow adds a stable map key and safe web link to one node declaration.
type webInstanceRow struct {
	webJobNode        // last declaration and receipt timestamp
	Name       string // registry key, stable even if the declaration is incomplete
	URL        string // safe node web URL, empty if unavailable
	Offline    bool   // heartbeat age is greater than or equal to the configured timeout
	NeverSeen  bool   // node was never seen (LastSeen is zero)
	Seen       string // precise last heartbeat time, empty when not known
}

// safeJobURL admits absolute HTTP(S) links only; values come from remote node declarations.
func safeJobURL(raw string) string {
	u, err := url.Parse(raw)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" || u.User != nil {
		return ""
	}
	return u.String()
}

// jobNodeURL links to a node task when known, otherwise to its web interface.
func jobNodeURL(raw, task string) string {
	base := safeJobURL(raw)
	if base == "" || task == "" {
		return base
	}
	u, err := url.Parse(base)
	if err != nil {
		return ""
	}
	u.RawQuery, u.Fragment = "", ""
	return strings.TrimRight(u.String(), "/") + "/task/" + url.PathEscape(task)
}

// webNodeOffline uses the same inclusive deadline as the lead registry.
func webNodeOffline(node webJobNode, now time.Time, ttl time.Duration) bool {
	return node.LastSeen.IsZero() || now.Sub(node.LastSeen) >= ttl
}

// viewJobs renders fresh public job records; it never alters the queue or contacts a remote installation.
func (s *webServer) viewJobs(w http.ResponseWriter, r *http.Request) {
	snapshot, err := readWebJobs(s.cfg.busDir)
	if err != nil {
		viewError(s, w, r, err)
		return
	}
	rows := make([]webJobRow, 0, len(snapshot.Jobs))
	now := time.Now()
	localSnap := s.snapshot()
	for _, job := range snapshot.Jobs {
		rows = append(rows, buildWebJobRow(job, snapshot, localSnap, now, s.cfg.nodeTTL))
	}
	s.render(w, r, http.StatusOK, "jobs", "Jobs", localSnap, rows)
}

// webInstanceList holds the filtered rows and preserves the operator's choice during refresh.
type webInstanceList struct {
	Rows    []webInstanceRow // visible nodes, sorted by name
	ShowAll bool             // include nodes whose known heartbeat is older than the visibility window
}

// instanceVisibilityWindow limits clutter without removing nodes from the registry.
const instanceVisibilityWindow = 24 * time.Hour

// viewInstances renders node capabilities, free agents and current heartbeat liveness.
func (s *webServer) viewInstances(w http.ResponseWriter, r *http.Request) {
	snapshot, err := readWebJobs(s.cfg.busDir)
	if err != nil {
		viewError(s, w, r, err)
		return
	}
	rows := make([]webInstanceRow, 0, len(snapshot.Nodes))
	showAll := r.URL.Query().Get("all") == "1"
	now := time.Now()
	for name, node := range snapshot.Nodes {
		neverSeen := node.LastSeen.IsZero()
		row := webInstanceRow{
			webJobNode: node,
			Name:       name,
			URL:        safeJobURL(node.Heartbeat.WebURL),
			NeverSeen:  neverSeen,
			Offline:    !neverSeen && webNodeOffline(node, now, s.cfg.nodeTTL),
		}
		if !showAll && row.Offline && now.Sub(node.LastSeen) > instanceVisibilityWindow {
			continue
		}
		if !node.LastSeen.IsZero() {
			row.Seen = node.LastSeen.Format(time.RFC3339Nano)
		}
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Name < rows[j].Name })
	s.render(w, r, http.StatusOK, "instances", "Instances", s.snapshot(), webInstanceList{Rows: rows, ShowAll: showAll})
}
