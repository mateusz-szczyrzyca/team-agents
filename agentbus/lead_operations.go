package main

import (
	"sort"
	"strings"
	"time"
)

// leadTransaction applies expiry and an operation under the durable registry's existing flock.
// fresh is the authenticated heartbeat caller, or empty for operations that do not refresh liveness.
func leadTransaction[C opCause](a *leadAPI, storage C, fresh string, fn func(*leadRegistry, Timestamp) *OpError[C]) *OpError[C] {
	// Capture receipt time inside the lock so queued lock wait cannot create a backwards transition.
	return transactJobs(a.bus, storage, Timestamp{time.Now().UTC()}, func(r *leadRegistry) *OpError[C] {
		now := Timestamp{time.Now().UTC()}
		// The callback owns this registry. Refresh the returning caller before expiry
		// can invalidate its pending start reports; rejected heartbeats roll back both.
		if node, ok := r.nodes[fresh]; fresh != "" && ok {
			node.LastSeen = now
			r.nodes[fresh] = node
		}
		r.requeueOffline(now, a.ttl)
		return fn(r, now)
	})
}

// submit binds delegated requester data to its authenticated device.
func (a *leadAPI) submit(node string, local bool, req JobRequest) (Job, *LeadSubmitError) {
	if req.Requester == "human" && !local || req.Requester != "human" && validJobRequester(req.Requester) && !strings.HasSuffix(req.Requester, "@"+node) {
		return Job{}, opErr(LeadSubmitUnauthorized, "requester must belong to the authenticated node; human submissions are lead-local")
	}
	var job Job // snapshot returned only after successful persistence
	err := leadTransaction(a, LeadSubmitStorage, "", func(r *leadRegistry, now Timestamp) *LeadSubmitError {
		var failure *LeadSubmitError // exact validation/conflict cause from the registry
		job, failure = r.submit(req, now)
		return failure
	})
	return job, err
}

// heartbeat applies one node's reports and declaration in one storage transaction.
func (a *leadAPI) heartbeat(node string, body leadHeartbeatBody) (*Job, *LeadHeartbeatError) {
	if body.Node != "" && body.Node != node {
		return nil, opErr(LeadHeartbeatUnauthorized, "heartbeat identity differs from its socket peer")
	}
	body.Node = node
	var job *Job // one assignment, or nil when no job matches
	err := leadTransaction(a, LeadHeartbeatStorage, node, func(r *leadRegistry, now Timestamp) *LeadHeartbeatError {
		for _, rep := range body.Reports {
			if rep.Node != "" && rep.Node != node {
				return opErr(LeadHeartbeatUnauthorized, "report identity differs from its socket peer")
			}
			rep.Node = node
			if _, err := r.report(rep, now); err != nil {
				return heartbeatReportError(err)
			}
		}
		var failure *LeadHeartbeatError // declaration validation or persistence failure
		job, failure = r.heartbeat(body.NodeHeartbeat, now)
		return failure
	})
	return job, err
}

// heartbeatReportError explicitly maps the shared report implementation to the heartbeat's own causes.
func heartbeatReportError(err *LeadReportError) *LeadHeartbeatError {
	cause := LeadHeartbeatInvalidState
	switch err.Cause {
	case LeadReportUnauthorized:
		cause = LeadHeartbeatUnauthorized
	case LeadReportNoSuchJob:
		cause = LeadHeartbeatNoSuchJob
	case LeadReportStaleAssignment:
		cause = LeadHeartbeatStaleAssignment
	case LeadReportTerminalConflict:
		cause = LeadHeartbeatTerminalConflict
	case LeadReportMalformedBody:
		cause = LeadHeartbeatMalformedBody
	case LeadReportStorage:
		cause = LeadHeartbeatStorage
	case LeadReportInvalidState:
		cause = LeadHeartbeatInvalidState
	}
	return opErr(cause, "%s", err.Message)
}

// report binds path and socket identities before invoking the shared report implementation.
func (a *leadAPI) report(node, id string, rep JobReport) (Job, *LeadReportError) {
	if rep.ID != "" && rep.ID != id {
		return Job{}, opErr(LeadReportMalformedBody, "report ID differs from the route")
	}
	if rep.Node != "" && rep.Node != node {
		return Job{}, opErr(LeadReportUnauthorized, "report identity differs from its socket peer")
	}
	rep.ID, rep.Node = id, node
	var job Job // durable accepted snapshot
	err := leadTransaction(a, LeadReportStorage, "", func(r *leadRegistry, now Timestamp) *LeadReportError {
		var failure *LeadReportError // shared lifecycle/assignment validation
		job, failure = r.report(rep, now)
		return failure
	})
	return job, err
}

// cancel accepts only the trusted lead-local caller.
func (a *leadAPI) cancel(local bool, id string) (Job, *LeadCancelError) {
	if !local {
		return Job{}, opErr(LeadCancelUnauthorized, "cancel jobs on the lead's loopback interface")
	}
	var job Job // actual state; started jobs only acquire a stop request
	err := leadTransaction(a, LeadCancelStorage, "", func(r *leadRegistry, now Timestamp) *LeadCancelError {
		var failure *LeadCancelError // missing job or storage failure
		job, failure = r.cancel(id, now)
		return failure
	})
	return job, err
}

// publicLeadJob omits internal sequence counters while preserving precise history times.
func publicLeadJob(job Job) leadJobView {
	history := make([]storedTransition, 0, len(job.History))
	for _, tr := range job.History {
		history = append(history, storedTransition{State: tr.State, At: jobTime{tr.At}})
	}
	return leadJobView{Request: job.Request, State: job.State, Node: job.Node, Assignment: job.Assignment, History: history, TaskID: job.TaskID, Branch: job.Branch, PRURL: job.PRURL, Result: job.Result, StopRequested: job.StopRequested}
}

// registryLeadJob adds live diagnostics to a detached public job snapshot.
func registryLeadJob(r *leadRegistry, job Job, now Timestamp, ttl time.Duration) leadJobView {
	view := publicLeadJob(job)
	view.WaitReason = r.waitReason(job.Request.ID, now, ttl)
	view.NodeOffline = job.Node != "" && r.nodeOffline(job.Node, now, ttl)
	return view
}

// get returns one public job from durable storage.
func (a *leadAPI) get(id string) (leadJobView, *LeadGetJobError) {
	var view leadJobView // result from one consistent registry transaction
	err := leadTransaction(a, LeadGetJobStorage, "", func(r *leadRegistry, now Timestamp) *LeadGetJobError {
		job, err := r.get(id)
		if err != nil {
			return err
		}
		view = registryLeadJob(r, job, now, a.ttl)
		return nil
	})
	return view, err
}

// listJobs returns ordered public jobs and current waiting diagnostics.
func (a *leadAPI) listJobs(node string) ([]leadJobView, *LeadListJobsError) {
	if node != "" && !validJobIdentifier(node) {
		return nil, opErr(LeadListJobsInvalidArgument, "node filter must be a logical instance name")
	}
	views := []leadJobView{}
	err := leadTransaction(a, LeadListJobsStorage, "", func(r *leadRegistry, now Timestamp) *LeadListJobsError {
		for _, id := range r.order {
			job := cloneJob(*r.jobs[id])
			if node == "" || job.Node == node {
				views = append(views, registryLeadJob(r, job, now, a.ttl))
			}
		}
		return nil
	})
	return views, err
}

// listNodes returns public node declarations in stable name order.
func (a *leadAPI) listNodes() ([]leadNodeView, *LeadListNodesError) {
	views := []leadNodeView{}
	err := leadTransaction(a, LeadListNodesStorage, "", func(r *leadRegistry, now Timestamp) *LeadListNodesError {
		for name, node := range r.nodes {
			views = append(views, leadNodeView{NodeHeartbeat: node.Heartbeat, LastSeen: jobTime{node.LastSeen}, Offline: r.nodeOffline(name, now, a.ttl)})
		}
		sort.Slice(views, func(i, j int) bool { return views[i].Node < views[j].Node })
		return nil
	})
	return views, err
}
