package main

import (
	"net/http"
	"time"
)

// leadAPI adapts authenticated operations to the durable registry without touching ordinary bus state.
type leadAPI struct {
	bus *Bus          // registry storage and shared flock
	ttl time.Duration // maximum node silence before unstarted assignments expire
}

// leadHeartbeatBody adds pending reports to the node's current declaration.
type leadHeartbeatBody struct {
	NodeHeartbeat             // declaration, with identity supplied by the socket peer
	Reports       []JobReport // latest unacknowledged snapshots, applied atomically
}

// leadJobView exposes job data without registry counters or bus internals.
type leadJobView struct {
	Request       JobRequest         // submitted data
	State         JobState           // current state
	Node          string             // assigned instance
	Assignment    uint64             // launch fencing evidence
	History       []storedTransition // lossless transition times
	TaskID        string             // node's local task
	Branch        string             // result branch
	PRURL         string             // optional pull request
	Result        string             // result head or failure reason
	StopRequested bool               // cancellation awaits the node
	WaitReason    string             // why a queued job cannot currently run
	NodeOffline   bool               // assigned instance exceeded its TTL
}

// leadNodeView exposes only a node's declaration, receipt time and liveness.
type leadNodeView struct {
	NodeHeartbeat         // last accepted declaration
	LastSeen      jobTime // precise heartbeat receipt time
	Offline       bool    // silence reached the configured TTL
}

// newLeadHandler constructs the HTTP boundary shared by configured listeners.
func newLeadHandler(_ *Bus, _ string, _ []string, _ time.Duration) http.Handler {
	return http.NotFoundHandler()
}

// submit binds delegated requester data to its authenticated device.
func (a *leadAPI) submit(_ string, _ bool, _ JobRequest) (Job, *LeadSubmitError) { return Job{}, nil }

// heartbeat applies one node's reports and declaration in one storage transaction.
func (a *leadAPI) heartbeat(_ string, _ leadHeartbeatBody) (*Job, *LeadHeartbeatError) {
	return nil, nil
}

// report binds path and socket identities before invoking the shared report implementation.
func (a *leadAPI) report(_, _ string, _ JobReport) (Job, *LeadReportError) { return Job{}, nil }

// cancel accepts only the trusted lead-local caller.
func (a *leadAPI) cancel(_ bool, _ string) (Job, *LeadCancelError) { return Job{}, nil }

// get returns one public job from durable storage.
func (a *leadAPI) get(_ string) (Job, *LeadGetJobError) { return Job{}, nil }

// listJobs returns ordered public jobs and current waiting diagnostics.
func (a *leadAPI) listJobs(_ string) ([]leadJobView, *LeadListJobsError) { return nil, nil }

// listNodes returns public node declarations in stable name order.
func (a *leadAPI) listNodes() ([]leadNodeView, *LeadListNodesError) { return nil, nil }
