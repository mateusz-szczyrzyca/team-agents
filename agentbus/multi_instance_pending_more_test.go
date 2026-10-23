package main

import "testing"

// Companion roadmap for cases that the design (docs/multi-instance.md) requires and that
// multi_instance_pending_test.go does not yet name. Like that file, every test is a skipped
// placeholder; the owning round replaces the skip with executable RED/GREEN tests.

// TestLeadRegistryExpiredAssignmentCannotStart: an expired assignment is refused even when the same node gets a new assignment for the same job.
func TestLeadRegistryExpiredAssignmentCannotStart(t *testing.T) {
	t.Skip("pending: expired assignment cannot start, even after reassignment to the same node, round 3")
}

// TestLeadRegistryCancelUnacknowledgedAssignmentIsImmediate: cancelling an assigned but unacknowledged job stops it at once.
func TestLeadRegistryCancelUnacknowledgedAssignmentIsImmediate(t *testing.T) {
	t.Skip("pending: cancel of an assigned, unacknowledged job is immediate, round 3")
}

// TestLeadRegistryFinishedJobsKeptThirtyDays: finished jobs stay visible for 30 days and are then removed.
func TestLeadRegistryFinishedJobsKeptThirtyDays(t *testing.T) {
	t.Skip("pending: finished jobs are kept for 30 days, round 3")
}

// TestLeadRegistryAbsentJobsFileIsEmpty: a missing jobs.json is an empty registry, not an error.
func TestLeadRegistryAbsentJobsFileIsEmpty(t *testing.T) {
	t.Skip("pending: absent jobs.json reads as an empty registry, round 3")
}

// TestLeadRegistryCorruptJobsFileFailsStorage: a corrupt jobs.json fails with a typed storage cause and is never reset silently.
func TestLeadRegistryCorruptJobsFileFailsStorage(t *testing.T) {
	t.Skip("pending: corrupt jobs.json is a typed storage failure, never reset to empty, round 3")
}

// TestLeadRegistryStopRequestReturnedUntilAcknowledged: a durable stop request is returned in heartbeats until the node acknowledges it.
func TestLeadRegistryStopRequestReturnedUntilAcknowledged(t *testing.T) {
	t.Skip("pending: durable stop request is returned in heartbeats until the node acknowledges it, round 6")
}

// TestLeadRegistryLatestHeartbeatWinsForCapabilities: the capabilities of the latest heartbeat are the ones used for routing.
func TestLeadRegistryLatestHeartbeatWinsForCapabilities(t *testing.T) {
	t.Skip("pending: latest heartbeat's capabilities and free agents are used for routing, round 2")
}

// TestLeadHTTPLoopbackRangeRequiresIdentity: 127.0.0.2 is not local authority; it needs an authenticated nonlocal node identity.
func TestLeadHTTPLoopbackRangeRequiresIdentity(t *testing.T) {
	t.Skip("pending: 127.0.0.2 requires authenticated nonlocal node identity, never local authority; no blanket refusal, round 4")
}

// TestLeadNotifiesOnFinalState: the lead notifies when a job becomes done, failed or stopped.
func TestLeadNotifiesOnFinalState(t *testing.T) {
	t.Skip("pending: lead notifications on done, failed and stopped, round 7")
}

// TestLeadRegistryConcurrentHeartbeatsRace: concurrent heartbeats and submits keep the registry consistent under -race.
func TestLeadRegistryConcurrentHeartbeatsRace(t *testing.T) {
	t.Skip("pending: registry race test under -race, round 2")
}

// TestLeadHTTPListenerConcurrentRequestsRace: concurrent HTTP requests to the listener are race-free under -race.
func TestLeadHTTPListenerConcurrentRequestsRace(t *testing.T) {
	t.Skip("pending: listener race test under -race, round 4")
}
