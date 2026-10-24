package main

import (
	"reflect"
	"slices"
	"strings"
	"sync"
	"time"
)

// The lead's pure in-memory model of jobs, nodes, routing and lifecycle operations.
// job_store.go persists whole operations under the bus lock; HTTP adapters arrive in a later round.

// JobState is the lifecycle state of a remote job.
type JobState string

const (
	JobQueued   JobState = "queued"   // waiting for a node heartbeat that can take it
	JobAssigned JobState = "assigned" // given to one node, not yet reported as started
	JobRunning  JobState = "running"  // the assigned node reported its start
	JobWaiting  JobState = "waiting"  // running, blocked on a question or a human
	JobDone     JobState = "done"     // finished with a result
	JobFailed   JobState = "failed"   // finished without a usable result
	JobStopped  JobState = "stopped"  // stopped by a cancellation
)

// Valid reports whether s is one of the defined job states.
func (s JobState) Valid() bool {
	switch s {
	case JobQueued, JobAssigned, JobRunning, JobWaiting, JobDone, JobFailed, JobStopped:
		return true
	}
	return false
}

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown states with a *ValueError.
func (s *JobState) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, s, "job state").asError()
}

// JobTransition is one entry of a job's history: the state it entered and when the lead recorded it.
type JobTransition struct {
	State JobState  // the state entered
	At    Timestamp // when the lead recorded the transition (the submit or heartbeat time)
}

// JobRequest is a submission, as the lead receives it. Every field is compared on resubmission.
type JobRequest struct {
	ID         string   // submitter-chosen idempotency key, validated before use
	Repository string   // logical repository name, resolved through the node's NODE_REPOS
	BaseRef    string   // git ref name, validated before any git invocation
	BaseSHA    string   // exact commit (40 or 64 hex digits, either case); optional for "human", required for an agent
	Requires   []string // capabilities the node must declare, all of them
	Selector   string   // a tier (normal or worker) or one configured agent name
	Target     string   // optional node name the job is pinned to; empty means any node
	Task       string   // task text, at most 65536 bytes
	Requester  string   // "human" or "<agent>@<instance>", both parts valid identifiers; routing data only
}

// Job is a registered job as the registry holds it.
type Job struct {
	Request       JobRequest      // the submitted fields, copied when stored and when returned
	Seq           uint64          // submission order; the oldest queued job has the lowest value
	State         JobState        // current lifecycle state
	Node          string          // the node the job is assigned to; empty while queued
	Assignment    uint64          // identity of the current assignment, greater than 0 once assigned, never reused
	History       []JobTransition // every state entered, in order; queued first, then assigned
	TaskID        string          // node's local task ID
	Branch        string          // branch pushed by the node
	PRURL         string          // optional PR URL
	Result        string          // final result text or failure reason
	StopRequested bool            // a local cancellation awaits the assigned node's terminal report
}

// FreeAgent is a configured agent of a node that is free at the time of a heartbeat.
type FreeAgent struct {
	Name string // the configured agent name on that node
	Tier Tier   // the tier of that agent
}

// NodeHeartbeat is what a node declares in one heartbeat. It replaces the node's previous declaration.
type NodeHeartbeat struct {
	Node         string      // the authenticated node name (the short device name)
	Capabilities []string    // declared capabilities, validated like JobRequest.Requires
	FreeAgents   []FreeAgent // configured agents that are free now; empty when none is free
	WebURL       string      // the node's AGENT_WEB_URL, shown by the lead for waiting jobs; may be empty
}

// JobReport is a state report sent by an assigned node for one job.
type JobReport struct {
	ID          string          // job ID being reported
	Node        string          // authenticated reporting node name
	Assignment  uint64          // assignment evidence the report belongs to
	State       JobState        // target state being reported
	TaskID      string          // node's local task ID
	Branch      string          // branch pushed by the node
	PRURL       string          // optional PR URL
	Result      string          // final result text or failure reason
	At          Timestamp       // timestamp of the transition; defaults to now when zero
	Transitions []JobTransition // cumulative intermediate transitions observed by the node
}

// leadRegistry is the lead's job and node registry. Every method is atomic with respect to the others.
type leadRegistry struct {
	mu             sync.Mutex         // protects the entire registry, including returned snapshots
	jobs           map[string]*Job    // jobs by submitter ID; pointers never escape the lock
	order          []string           // submission order, including equal-timestamp submissions
	nodes          map[string]jobNode // latest accepted declaration of each authenticated node
	nextSequence   uint64             // next submission sequence; duplicates do not consume one
	nextAssignment uint64             // assignment evidence; never reused within this registry
}

// jobNode keeps trusted receipt time separate from the node's declaration.
type jobNode struct {
	Heartbeat NodeHeartbeat // cloned snapshot, replaced only after validation succeeds
	LastSeen  Timestamp     // lead's receipt time; never advanced by invalid heartbeats
}

// newLeadRegistry returns an empty registry.
func newLeadRegistry() *leadRegistry {
	return &leadRegistry{jobs: make(map[string]*Job), nodes: make(map[string]jobNode)}
}

// submit registers a job or returns the existing one with the same ID when the submission is identical.
// It returns a copy of the stored job. Validation failures and a different submission under an existing ID
// are refused; a refused submission never changes the registry.
func (r *leadRegistry) submit(req JobRequest, now Timestamp) (Job, *LeadSubmitError) {
	req, err := validateJobRequest(req)
	if err != nil {
		return Job{}, err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if old, ok := r.jobs[req.ID]; ok {
		if !reflect.DeepEqual(old.Request, req) {
			return Job{}, opErr(LeadSubmitJobConflict, "job %q already has a different submission", req.ID)
		}
		return cloneJob(*old), nil
	}
	r.nextSequence++
	job := &Job{Request: req, Seq: r.nextSequence, State: JobQueued,
		History: []JobTransition{{State: JobQueued, At: now}}}
	r.jobs[req.ID] = job
	r.order = append(r.order, req.ID)
	return cloneJob(*job), nil
}

// validateJobRequest normalizes set-valued requirements before identity comparison, without mutating input.
func validateJobRequest(req JobRequest) (JobRequest, *LeadSubmitError) {
	if req.ID == "" || req.Repository == "" || req.BaseRef == "" || req.Selector == "" ||
		strings.TrimSpace(req.Task) == "" || req.Requester == "" {
		return JobRequest{}, opErr(LeadSubmitMissingArgument, "id, repository, base ref, selector, task and requester are required")
	}
	if !validJobIdentifier(req.ID) {
		return JobRequest{}, opErr(LeadSubmitInvalidID, "job ID %q must be a bounded logical identifier", req.ID)
	}
	if !validJobBase(req.BaseRef, req.BaseSHA) || req.Requester != "human" && req.BaseSHA == "" {
		return JobRequest{}, opErr(LeadSubmitInvalidBase, "use a ref name and an exact SHA for delegated jobs")
	}
	if !validJobIdentifier(req.Repository) || !validJobIdentifier(req.Selector) ||
		req.Target != "" && !validJobIdentifier(req.Target) || !validJobRequester(req.Requester) {
		return JobRequest{}, opErr(LeadSubmitMalformedBody, "repository, selector, target and requester must be logical names")
	}
	if len(req.Task) > 65536 {
		return JobRequest{}, opErr(LeadSubmitTaskTooLarge, "task text exceeds 65536 bytes")
	}
	caps, err := canonicalJobCapabilities(req.Requires)
	if err != nil {
		return JobRequest{}, opErr(LeadSubmitInvalidCapabilities, "%v", err)
	}
	req.Requires = caps
	req.BaseSHA = strings.ToLower(req.BaseSHA)
	return req, nil
}

// validJobRequester checks routing data only; the transport separately binds an agent to its submitting node.
func validJobRequester(s string) bool {
	if s == "human" {
		return true
	}
	agent, node, found := strings.Cut(s, "@")
	return found && validJobIdentifier(agent) && validJobIdentifier(node)
}

// canonicalJobCapabilities produces an owned sorted set; nil and empty input have the same identity.
func canonicalJobCapabilities(raw []string) ([]string, *CapsParseError) {
	caps, err := parseCapabilities(raw)
	if err != nil {
		return nil, err
	}
	var result []string
	for _, capability := range caps {
		result = append(result, capability.String())
	}
	return result, nil
}

// cloneJob keeps mutable slices owned by the registry even after a result leaves its lock.
func cloneJob(job Job) Job {
	job.Request.Requires = slices.Clone(job.Request.Requires)
	job.History = slices.Clone(job.History)
	return job
}

// heartbeat records the node's declaration at time now and returns the job assigned to it, or nil when none
// fits. A node with an unacknowledged assignment gets that same assignment back (a lost reply is retried,
// never a second job). A job is assigned at most once, and only to one heartbeat that matches it.
func (r *leadRegistry) heartbeat(h NodeHeartbeat, now Timestamp) (*Job, *LeadHeartbeatError) {
	if !validJobIdentifier(h.Node) {
		return nil, opErr(LeadHeartbeatMalformedBody, "heartbeat needs a valid node name")
	}
	caps, err := canonicalJobCapabilities(h.Capabilities)
	if err != nil {
		return nil, opErr(LeadHeartbeatInvalidCapabilities, "%v", err)
	}
	seen := make(map[string]bool, len(h.FreeAgents))
	for _, agent := range h.FreeAgents {
		if !validJobIdentifier(agent.Name) || !agent.Tier.Valid() || seen[agent.Name] {
			return nil, opErr(LeadHeartbeatMalformedBody, "free agents need unique configured names and known tiers")
		}
		seen[agent.Name] = true
	}
	h.Capabilities = caps
	h.FreeAgents = slices.Clone(h.FreeAgents)
	r.mu.Lock()
	defer r.mu.Unlock()
	r.nodes[h.Node] = jobNode{Heartbeat: h, LastSeen: now}
	if job := r.outstanding(h.Node); job != nil {
		snapshot := cloneJob(*job)
		return &snapshot, nil
	}
	for _, id := range r.order {
		job := r.jobs[id]
		if job.State != JobQueued || !jobNodeMatches(job.Request, h) {
			continue
		}
		r.nextAssignment++
		job.State, job.Node, job.Assignment = JobAssigned, h.Node, r.nextAssignment
		job.History = append(job.History, JobTransition{State: JobAssigned, At: atOrAfterLast(job, now)})
		snapshot := cloneJob(*job)
		return &snapshot, nil
	}
	return nil, nil
}

// outstanding finds only unstarted assignments; callers hold mu. Running jobs must never be offered for launch.
func (r *leadRegistry) outstanding(node string) *Job {
	for _, id := range r.order {
		job := r.jobs[id]
		if job.Node == node && job.State == JobAssigned {
			return job
		}
	}
	return nil
}

// jobNodeMatches uses the same conjunction for assignment and its diagnostic, including any node pin.
func jobNodeMatches(req JobRequest, h NodeHeartbeat) bool {
	return (req.Target == "" || req.Target == h.Node) && jobHasCapabilities(h.Capabilities, req.Requires) &&
		jobHasFreeAgent(h.FreeAgents, req.Selector)
}

// jobHasCapabilities compares canonical strings; all inputs were validated before storage.
func jobHasCapabilities(available, required []string) bool {
	for _, capability := range required {
		if !slices.Contains(available, capability) {
			return false
		}
	}
	return true
}

// jobHasFreeAgent checks capacity, leaving the actual configured agent selection to the node.
func jobHasFreeAgent(free []FreeAgent, selector string) bool {
	tier, err := parseEnum[Tier](selector, "agent tier")
	for _, agent := range free {
		if err == nil && agent.Tier == tier || err != nil && agent.Name == selector {
			return true
		}
	}
	return false
}

// waitReason explains why the queued job id is not assigned, naming the first blocker: a required capability
// no live node declares ("no node with hw:rp2350"), or no live node with a free agent of the selector's tier
// ("no free normal agent"). A node is live when now minus its last heartbeat is less than ttl; at ttl or more
// it is offline and does not count. A node with an unstarted assignment cannot take another job, so it does not
// count as free either. It returns "" when the job is not queued or no blocker is known.
func (r *leadRegistry) waitReason(id string, now Timestamp, ttl time.Duration) string {
	r.mu.Lock()
	defer r.mu.Unlock()
	job, ok := r.jobs[id]
	if !ok || job.State != JobQueued {
		return ""
	}
	var live []NodeHeartbeat
	for _, node := range r.nodes {
		if now.Sub(node.LastSeen.Time) < ttl && (job.Request.Target == "" || node.Heartbeat.Node == job.Request.Target) {
			live = append(live, node.Heartbeat)
		}
	}
	for _, capability := range job.Request.Requires {
		found := false
		for _, node := range live {
			if slices.Contains(node.Capabilities, capability) {
				found = true
				break
			}
		}
		if !found {
			return "no node with " + capability
		}
	}
	capable := false
	for _, node := range live {
		if !jobHasCapabilities(node.Capabilities, job.Request.Requires) {
			continue
		}
		capable = true
		if r.outstanding(node.Node) == nil && jobNodeMatches(job.Request, node) {
			return ""
		}
	}
	if !capable && len(job.Request.Requires) > 0 {
		return "no node with " + strings.Join(job.Request.Requires, " and ")
	}
	return "no free " + job.Request.Selector + " agent"
}

// checkAssignment decides whether a state report from node about job id under assignment may be accepted.
// It returns nil only for the node and the assignment currently recorded for the job.
func (r *leadRegistry) checkAssignment(id, node string, assignment uint64) *LeadReportError {
	r.mu.Lock()
	defer r.mu.Unlock()
	job, ok := r.jobs[id]
	if !ok {
		return opErr(LeadReportNoSuchJob, "no job %q", id)
	}
	if job.State == JobQueued {
		if assignment > 0 {
			return opErr(LeadReportStaleAssignment, "job %q assignment %d has expired", id, assignment)
		}
		return opErr(LeadReportUnauthorized, "job %q is not assigned", id)
	}
	if job.Node != node {
		return opErr(LeadReportUnauthorized, "job %q is not assigned to node %q", id, node)
	}
	if assignment == 0 || job.Assignment != assignment {
		return opErr(LeadReportStaleAssignment, "job %q has a different assignment", id)
	}
	return nil
}

// atOrAfterLast returns at, raised to the job's last recorded transition when at is earlier. The lead's clock can
// move backwards and a time may be captured before waiting for the lock; the history must never decrease, or the
// store refuses to load it. Callers hold mu.
func atOrAfterLast(job *Job, at Timestamp) Timestamp {
	last := job.History[len(job.History)-1].At
	if at.Before(last.Time) {
		return last
	}
	return at
}

// validTransition reports whether a lifecycle transition from state to next is allowed.
func validTransition(from, to JobState) bool {
	switch from {
	case JobAssigned:
		return to == JobRunning || to == JobFailed || to == JobStopped
	case JobRunning:
		return to == JobRunning || to == JobWaiting || to == JobDone || to == JobFailed || to == JobStopped
	case JobWaiting:
		return to == JobWaiting || to == JobRunning || to == JobDone || to == JobFailed || to == JobStopped
	case JobDone, JobFailed, JobStopped:
		return from == to
	}
	return false
}

// isTerminalJob reports whether s is a terminal lifecycle state.
func isTerminalJob(s JobState) bool {
	return s == JobDone || s == JobFailed || s == JobStopped
}

// report records a state transition reported by the assigned node for job.
func (r *leadRegistry) report(rep JobReport, now Timestamp) (Job, *LeadReportError) {
	r.mu.Lock()
	defer r.mu.Unlock()
	job, ok := r.jobs[rep.ID]
	if !ok {
		return Job{}, opErr(LeadReportNoSuchJob, "no job %q", rep.ID)
	}
	if job.State == JobQueued {
		if rep.Assignment > 0 {
			return Job{}, opErr(LeadReportStaleAssignment, "job %q assignment %d has expired", rep.ID, rep.Assignment)
		}
		return Job{}, opErr(LeadReportUnauthorized, "job %q is not assigned", rep.ID)
	}
	if job.Node != rep.Node {
		return Job{}, opErr(LeadReportUnauthorized, "job %q is not assigned to node %q", rep.ID, rep.Node)
	}
	if rep.Assignment == 0 || job.Assignment != rep.Assignment {
		return Job{}, opErr(LeadReportStaleAssignment, "job %q has a different assignment", rep.ID)
	}

	// A cumulative report carries its own time, which must be the time of its last transition, so the first
	// acceptance and an identical retry judge the same payload.
	if len(rep.Transitions) > 0 && !rep.At.IsZero() && !rep.At.Equal(rep.Transitions[len(rep.Transitions)-1].At.Time) {
		return Job{}, opErr(LeadReportInvalidState, "report time %v differs from its last transition %v", rep.At, rep.Transitions[len(rep.Transitions)-1].At)
	}

	if isTerminalJob(job.State) {
		lastTr := job.History[len(job.History)-1]
		if job.State != rep.State || job.Result != rep.Result ||
			(rep.TaskID != "" && job.TaskID != rep.TaskID) ||
			(rep.Branch != "" && job.Branch != rep.Branch) ||
			(rep.PRURL != "" && job.PRURL != rep.PRURL) ||
			(!rep.At.IsZero() && !rep.At.Equal(lastTr.At.Time)) {
			return Job{}, opErr(LeadReportTerminalConflict, "job %q is terminal with %s, cannot revise", rep.ID, job.State)
		}
		if len(rep.Transitions) > 0 {
			assignedIdx := -1
			for i := len(job.History) - 1; i >= 0; i-- {
				if job.History[i].State == JobAssigned {
					assignedIdx = i
					break
				}
			}
			var recorded []JobTransition
			if assignedIdx >= 0 && assignedIdx+1 < len(job.History) {
				recorded = job.History[assignedIdx+1:]
			}
			if len(rep.Transitions) != len(recorded) {
				return Job{}, opErr(LeadReportTerminalConflict, "terminal retry transitions count %d differs from %d", len(rep.Transitions), len(recorded))
			}
			for i := range recorded {
				if rep.Transitions[i].State != recorded[i].State || !rep.Transitions[i].At.Equal(recorded[i].At.Time) {
					return Job{}, opErr(LeadReportTerminalConflict, "terminal retry transition at index %d differs", i)
				}
			}
		}
		return cloneJob(*job), nil
	}

	newHistory := slices.Clone(job.History)
	targetState := job.State

	if len(rep.Transitions) > 0 {
		if rep.Transitions[len(rep.Transitions)-1].State != rep.State {
			return Job{}, opErr(LeadReportInvalidState, "cumulative history does not end at reported state %s", rep.State)
		}
		assignedIdx := -1
		for i := len(job.History) - 1; i >= 0; i-- {
			if job.History[i].State == JobAssigned {
				assignedIdx = i
				break
			}
		}
		var recorded []JobTransition
		if assignedIdx >= 0 && assignedIdx+1 < len(job.History) {
			recorded = job.History[assignedIdx+1:]
		}
		if len(rep.Transitions) < len(recorded) {
			return Job{}, opErr(LeadReportInvalidState, "cumulative history regresses previously recorded transitions")
		}
		for i := 0; i < len(recorded); i++ {
			if rep.Transitions[i].State != recorded[i].State || !rep.Transitions[i].At.Equal(recorded[i].At.Time) {
				return Job{}, opErr(LeadReportInvalidState, "cumulative history conflicts with recorded history at index %d", i)
			}
		}
		newTransitions := rep.Transitions[len(recorded):]
		currState := job.State
		lastAt := job.History[len(job.History)-1].At
		for _, tr := range newTransitions {
			if tr.At.IsZero() || tr.At.Before(lastAt.Time) {
				return Job{}, opErr(LeadReportInvalidState, "transition timestamp %v must be nonzero and nondecreasing after %v", tr.At, lastAt)
			}
			if !validTransition(currState, tr.State) {
				return Job{}, opErr(LeadReportInvalidState, "invalid transition from %s to %s", currState, tr.State)
			}
			newHistory = append(newHistory, tr)
			currState = tr.State
			lastAt = tr.At
		}
		targetState = currState
	} else {
		at := rep.At
		if at.IsZero() {
			at = atOrAfterLast(job, now)
		}
		lastAt := job.History[len(job.History)-1].At
		if at.Before(lastAt.Time) {
			return Job{}, opErr(LeadReportInvalidState, "report timestamp %v cannot precede last transition %v", at, lastAt)
		}
		switch {
		case job.State == rep.State:
			// A same-state report may fill in a field that is still empty, but must not replace an identifier
			// already recorded: an older retry would otherwise overwrite a newer payload (investigator #27, R2).
			if (rep.Branch != "" && job.Branch != "" && rep.Branch != job.Branch) ||
				(rep.TaskID != "" && job.TaskID != "" && rep.TaskID != job.TaskID) ||
				(rep.PRURL != "" && job.PRURL != "" && rep.PRURL != job.PRURL) {
				return Job{}, opErr(LeadReportInvalidState, "state %s already records a different branch, task or PR for job %q", rep.State, rep.ID)
			}
			// A repeated waiting report with its own later time is a new wait for a human. Anything else that
			// repeats the current state (an identical retry, a report without a time) changes nothing.
			if job.State == JobWaiting && !rep.At.IsZero() && at.After(lastAt.Time) {
				newHistory = append(newHistory, JobTransition{State: JobWaiting, At: at})
			}
		case rep.At.IsZero() && job.State != JobAssigned && !isTerminalJob(rep.State):
			// Only the first start or a terminal state may omit its time: neither can rewind a job, since a
			// job becomes terminal once. Other changes without a time cannot be told from a replay of an older
			// report, so the node must send the time of every transition.
			return Job{}, opErr(LeadReportInvalidState, "state change %s from %s needs its transition time; a report without one cannot be told from a replay", rep.State, job.State)
		case at.Equal(lastAt.Time):
			// Two states at one instant cannot be ordered; the node must send the cumulative history instead.
			return Job{}, opErr(LeadReportInvalidState, "state %s at %v cannot follow %s at the same instant; send the cumulative history", rep.State, at, job.State)
		default:
			if !validTransition(job.State, rep.State) {
				return Job{}, opErr(LeadReportInvalidState, "invalid transition from %s to %s", job.State, rep.State)
			}
			targetState = rep.State
			newHistory = append(newHistory, JobTransition{State: rep.State, At: at})
		}
	}

	job.State = targetState
	job.History = newHistory
	if isTerminalJob(targetState) {
		// A real terminal result acknowledges the stop request, including completion winning the race.
		job.StopRequested = false
	}

	if rep.TaskID != "" {
		job.TaskID = rep.TaskID
	}
	if rep.Branch != "" {
		job.Branch = rep.Branch
	}
	if rep.PRURL != "" {
		job.PRURL = rep.PRURL
	}
	// A terminal report carries the final payload, so its result replaces any intermediate one, even an empty one.
	if rep.Result != "" || isTerminalJob(targetState) {
		job.Result = rep.Result
	}

	return cloneJob(*job), nil
}

// requeueOffline returns unstarted assigned jobs of offline nodes to the queue.
// Started jobs (running, waiting) remain in their state and are never requeued.
func (r *leadRegistry) requeueOffline(now Timestamp, ttl time.Duration) []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	var requeued []string
	for _, id := range r.order {
		job := r.jobs[id]
		if job.State != JobAssigned {
			continue
		}
		node, ok := r.nodes[job.Node]
		if !ok || now.Sub(node.LastSeen.Time) >= ttl {
			job.State = JobQueued
			job.Node = ""
			job.History = append(job.History, JobTransition{State: JobQueued, At: atOrAfterLast(job, now)})
			requeued = append(requeued, id)
		}
	}
	return requeued
}

// nodeOffline reports whether node has missed its heartbeat deadline at now.
func (r *leadRegistry) nodeOffline(node string, now Timestamp, ttl time.Duration) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	n, ok := r.nodes[node]
	if !ok {
		return true
	}
	return now.Sub(n.LastSeen.Time) >= ttl
}

// get returns a snapshot copy of job id.
func (r *leadRegistry) get(id string) (Job, *LeadGetJobError) {
	r.mu.Lock()
	defer r.mu.Unlock()
	job, ok := r.jobs[id]
	if !ok {
		return Job{}, opErr(LeadGetJobNoSuchJob, "no job %q", id)
	}
	return cloneJob(*job), nil
}
