// What a team waits for. A member asleep while a worker (delegate) or an investigator works on its request is not
// stuck: the result wakes it. The stuck detection of the agent script asks `agentbus team-waiting` before it calls a
// team stuck (bin/agent: team_after_run).
package main

// open reports whether a worker request has not finished yet (its result is still to come).
func (s HelperStatus) open() bool {
	return false // not implemented yet: teamwait_test.go comes first
}

// open reports whether an investigation has not finished yet (its result is still to come).
func (s InvestigationStatus) open() bool {
	return false // not implemented yet
}

// teamWaitingOn returns what the members of a team wait for — their unfinished worker requests and investigations —
// as "H12 (worker request of lead, running)" and "#7 (investigation of member, queued)", sorted. Nil: nothing.
func teamWaitingOn(st *State, team string) []string {
	return nil // not implemented yet
}
