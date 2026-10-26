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
	Request webJobRequest // public request identity and requested agent selector
	State   string        // lifecycle label, displayed without executing state transitions
	Node    string        // assigned node, empty while queued
	TaskID  string        // task ID on the assigned node
	Branch  string        // branch pushed by the task
	PRURL   string        // optional pull request URL, sanitized before use as a link
	Result  string        // final summary or provisioning diagnostic
}

// webJobRequest holds public submission fields useful when scanning jobs.
type webJobRequest struct {
	ID         string   // submitter-chosen job ID
	Repository string   // logical repository name
	Selector   string   // requested configured agent name or tier
	Requires   []string // required capabilities
}

// webJobNode is the read-only declaration and trusted heartbeat receipt time.
type webJobNode struct {
	Heartbeat webJobHeartbeat // last accepted node declaration
	LastSeen  time.Time       // precise lead receipt time in RFC3339Nano
}

// webJobHeartbeat contains the fields the lead may expose to a browser.
type webJobHeartbeat struct {
	Node         string        // authenticated node name
	Capabilities []string      // capabilities declared by the operator
	FreeAgents   []webJobAgent // agents free at the last accepted heartbeat
	WebURL       string        // operator-supplied address of the node's web interface
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
	webJob         // persisted public fields
	NodeURL string // safe node web/task URL, empty if unavailable
	Offline bool   // assigned node is missing or has reached its heartbeat deadline
}

// webInstanceRow adds a stable map key and safe web link to one node declaration.
type webInstanceRow struct {
	webJobNode        // last declaration and receipt timestamp
	Name       string // registry key, stable even if the declaration is incomplete
	URL        string // safe node web URL, empty if unavailable
	Offline    bool   // heartbeat age is greater than or equal to the configured timeout
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
	for _, job := range snapshot.Jobs {
		node := snapshot.Nodes[job.Node]
		job.PRURL = safeJobURL(job.PRURL)
		rows = append(rows, webJobRow{webJob: job, NodeURL: jobNodeURL(node.Heartbeat.WebURL, job.TaskID), Offline: job.Node != "" && webNodeOffline(node, now, s.cfg.nodeTTL)})
	}
	s.render(w, r, http.StatusOK, "jobs", "Jobs", s.snapshot(), rows)
}

// viewInstances renders node capabilities, free agents and current heartbeat liveness.
func (s *webServer) viewInstances(w http.ResponseWriter, r *http.Request) {
	snapshot, err := readWebJobs(s.cfg.busDir)
	if err != nil {
		viewError(s, w, r, err)
		return
	}
	rows := make([]webInstanceRow, 0, len(snapshot.Nodes))
	now := time.Now()
	for name, node := range snapshot.Nodes {
		row := webInstanceRow{webJobNode: node, Name: name, URL: safeJobURL(node.Heartbeat.WebURL), Offline: webNodeOffline(node, now, s.cfg.nodeTTL)}
		if !node.LastSeen.IsZero() {
			row.Seen = node.LastSeen.Format(time.RFC3339Nano)
		}
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Name < rows[j].Name })
	s.render(w, r, http.StatusOK, "instances", "Instances", s.snapshot(), rows)
}
