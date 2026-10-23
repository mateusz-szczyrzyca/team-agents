package main

import "time"

// Remote job registry of the lead (round 2 of docs/multi-instance.md): the pure in-memory model of jobs,
// nodes, routing and assignment. Persistence (jobs.json), state reports and HTTP come in later rounds.
// This file holds the RED signatures; the bodies are stubs until the GREEN commit implements them.

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

// JobRequest is a submission, as the lead receives it. Every field is compared on resubmission.
type JobRequest struct {
	ID         string   // submitter-chosen idempotency key, validated before use
	Repository string   // logical repository name, resolved through the node's NODE_REPOS
	BaseRef    string   // git ref name, validated before any git invocation
	BaseSHA    string   // exact commit (40 or 64 lowercase hex digits)
	Requires   []string // capabilities the node must declare, all of them
	Selector   string   // a tier (normal or worker) or one configured agent name
	Target     string   // optional node name the job is pinned to; empty means any node
	Task       string   // task text, at most 65536 bytes
	Requester  string   // "human" or "<agent>@<instance>", routing data only
}

// Job is a registered job as the registry holds it.
type Job struct {
	Request    JobRequest // the submitted fields, copied when stored and when returned
	Seq        uint64     // submission order; the oldest queued job has the lowest value
	State      JobState   // current lifecycle state
	Node       string     // the node the job is assigned to; empty while queued
	Assignment uint64     // identity of the current assignment, greater than 0 once assigned, never reused
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
}

// leadRegistry is the lead's job and node registry. Every method is atomic with respect to the others.
type leadRegistry struct{}

// newLeadRegistry returns an empty registry.
func newLeadRegistry() *leadRegistry { return &leadRegistry{} }

// submit registers a job or returns the existing one with the same ID when the submission is identical.
// It returns a copy of the stored job. Validation failures and a different submission under an existing ID
// are refused; a refused submission never changes the registry.
func (r *leadRegistry) submit(_ JobRequest, _ Timestamp) (Job, *LeadSubmitError) {
	return Job{}, nil
}

// heartbeat records the node's declaration at time now and returns the job assigned to it, or nil when none
// fits. A node with an unacknowledged assignment gets that same assignment back (a lost reply is retried,
// never a second job). A job is assigned at most once, and only to one heartbeat that matches it.
func (r *leadRegistry) heartbeat(_ NodeHeartbeat, _ Timestamp) (*Job, *LeadHeartbeatError) {
	return nil, nil
}

// waitReason explains why the queued job id is not assigned, naming the first blocker: a required capability
// no live node declares ("no node with hw:rp2350"), or no live node with a free agent of the selector's tier
// ("no free normal agent"). A node is live when its last heartbeat is at most ttl before now. It returns ""
// when the job is not queued or no blocker is known.
func (r *leadRegistry) waitReason(_ string, _ Timestamp, _ time.Duration) string {
	return ""
}

// checkAssignment decides whether a state report from node about job id under assignment may be accepted.
// It returns nil only for the node and the assignment currently recorded for the job.
func (r *leadRegistry) checkAssignment(_, _ string, _ uint64) *LeadReportError {
	return nil
}
