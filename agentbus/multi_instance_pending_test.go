package main

import "testing"

// Pending acceptance roadmap for the multi-instance jobs (task 20261023-151549-openrouter-haiku).
// Each test is a skipped placeholder for behaviour that a later round implements. A round removes
// the skips of its own tests and turns them into RED/GREEN tests. Nothing here may depend on code
// that does not exist yet; the names are the contract.

// TestLeadRegistrySubmitIsIdempotent: resubmitting the same job ID returns the existing job.
func TestLeadRegistrySubmitIsIdempotent(t *testing.T) {
	t.Skip("pending: submit with the same job ID is the same job (no duplicate), round 2")
}

// TestLeadRegistryHeartbeatAssignsOneJobAtomically: one heartbeat gets at most one job, never twice.
func TestLeadRegistryHeartbeatAssignsOneJobAtomically(t *testing.T) {
	t.Skip("pending: a heartbeat takes at most one job and a job is never assigned twice, round 2")
}

// TestLeadRegistryRoutesByCapabilityAndAgent: a job goes only to a node with all required capabilities and a free agent of its tier.
func TestLeadRegistryRoutesByCapabilityAndAgent(t *testing.T) {
	t.Skip("pending: routing by requires and free agent tier or name, oldest queued job first, round 2")
}

// TestLeadRegistryWaitReasonNamesBlocker: a queued job says why it waits ("no node with hw:rp2350", "no free normal agent").
func TestLeadRegistryWaitReasonNamesBlocker(t *testing.T) {
	t.Skip("pending: a queued job reports the exact reason it waits, round 2")
}

// TestLeadRegistryRejectsReportFromOtherNode: a state report from a node that is not assigned is refused with its cause.
func TestLeadRegistryRejectsReportFromOtherNode(t *testing.T) {
	t.Skip("pending: state reports accepted only from the assigned node, cause checked with errors.As, round 2")
}

// TestLeadRegistryRejectsSkippedTransition: assigned straight to done is refused; running must come first.
func TestLeadRegistryRejectsSkippedTransition(t *testing.T) {
	t.Skip("pending: transition table (assigned->running required before done), round 3")
}

// TestLeadRegistryOfflineUnstartedJobRequeued: an offline node's assigned but unacknowledged job goes back to the queue.
func TestLeadRegistryOfflineUnstartedJobRequeued(t *testing.T) {
	t.Skip("pending: offline node's assigned, unacknowledged job returns to queued, round 3")
}

// TestLeadRegistryOfflineStartedJobNeverRequeued: an acknowledged running job is never reassigned while its node is offline.
func TestLeadRegistryOfflineStartedJobNeverRequeued(t *testing.T) {
	t.Skip("pending: acknowledged running job is never requeued; shows node offline, round 3")
}

// TestLeadRegistryCancelQueuedIsImmediate: cancelling a queued job stops it at once.
func TestLeadRegistryCancelQueuedIsImmediate(t *testing.T) {
	t.Skip("pending: cancel of a queued or unacknowledged job is immediate, round 3")
}

// TestLeadRegistryCancelActiveWaitsForNodeConfirmation: an active job stays in its state until the node reports stopped.
func TestLeadRegistryCancelActiveWaitsForNodeConfirmation(t *testing.T) {
	t.Skip("pending: active cancel is a durable stop request until the node reports stopped, round 3")
}

// TestLeadRegistryReportHistoryKeepsFirstStart: cumulative report history keeps the first acknowledged start.
func TestLeadRegistryReportHistoryKeepsFirstStart(t *testing.T) {
	t.Skip("pending: repeated waiting reports and the first acknowledged start survive compaction, round 3")
}

// TestLeadRegistryRetryIsIdempotent: an identical report retry is accepted; a conflicting terminal revision is refused.
func TestLeadRegistryRetryIsIdempotent(t *testing.T) {
	t.Skip("pending: identical retries accepted, conflicting terminal revisions refused, round 3")
}

// TestLeadHTTPIdentityLoopbackIsLeadNode: a connection from 127.0.0.1 or ::1 is the lead's own node.
func TestLeadHTTPIdentityLoopbackIsLeadNode(t *testing.T) {
	t.Skip("pending: loopback peers are the lead's node, whois not consulted, round 4")
}

// TestLeadHTTPIdentityAllowsListedDeviceByNameAndTag: LEAD_NODES entries match device name or tag:….
func TestLeadHTTPIdentityAllowsListedDeviceByNameAndTag(t *testing.T) {
	t.Skip("pending: devices allowed by short name or tag from LEAD_NODES, round 4")
}

// TestLeadHTTPIdentityDeniesUnlistedDevice: a device not in LEAD_NODES is refused.
func TestLeadHTTPIdentityDeniesUnlistedDevice(t *testing.T) {
	t.Skip("pending: unlisted tailnet device refused, cause checked with errors.As, round 4")
}

// TestLeadHTTPIdentityFailsWhenWhoisNotStubbed: the test whois fake fails when a test forgets to stub it.
func TestLeadHTTPIdentityFailsWhenWhoisNotStubbed(t *testing.T) {
	t.Skip("pending: unstubbed whois fails the test, as in live/, round 4")
}

// TestNodeCapabilitiesRejectUnknownNamespace: an unknown capability namespace stops the node with a clear error.
func TestNodeCapabilitiesRejectUnknownNamespace(t *testing.T) {
	t.Skip("pending: capabilities outside os, arch, build, test, hw, net are refused, round 4")
}

// TestNodeTwoHomesRecoverAfterLeadRestart: a node keeps its job running and reports after the lead returns.
func TestNodeTwoHomesRecoverAfterLeadRestart(t *testing.T) {
	t.Skip("pending: lead stopped during a job, node reports after restart, two homes, round 6")
}

// TestNodeCheckoutFailureFailsJobWithReason: a failed clone or worktree fails the job with a typed reason; nothing is marked started.
func TestNodeCheckoutFailureFailsJobWithReason(t *testing.T) {
	t.Skip("pending: checkout failure is failed with reason, no running report sent, round 5")
}

// TestNodeRefusesRepositoryOutsideAllowList: a job naming a repository not in NODE_REPOS is refused.
func TestNodeRefusesRepositoryOutsideAllowList(t *testing.T) {
	t.Skip("pending: repository not in NODE_REPOS refused before any work, round 5")
}

// TestCLIRunNeedSubmitsJob: agent run --need submits a job instead of starting a local task.
func TestCLIRunNeedSubmitsJob(t *testing.T) {
	t.Skip("pending: run --need and --on submit a job through the lead, round 7")
}

// TestDelegateRequiresCreatesJob: a delegate request with requires the local node lacks becomes a job on the lead.
func TestDelegateRequiresCreatesJob(t *testing.T) {
	t.Skip("pending: delegate with requires becomes a job based on the pushed SHA, round 8")
}

// TestDelegateResultReturnsHelperMessage: the finished job reaches the member as the ordinary helper-result message.
func TestDelegateResultReturnsHelperMessage(t *testing.T) {
	t.Skip("pending: delegated job result goes back as a helper-result message, round 8")
}

// TestWebJobsAndInstancesViews: the lead's web interface shows jobs and instances; the node marks job tasks.
func TestWebJobsAndInstancesViews(t *testing.T) {
	t.Skip("pending: Jobs and Instances views, job marker on a node task, round 9")
}
