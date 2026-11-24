package main

// ownedCommand identifies the execution allowed to change a request or deliver its final notice.
type ownedCommand struct {
	ID      int64        `json:"id"`      // request number
	Action  string       `json:"action"`  // start, retry or settle
	Worker  string       `json:"worker"`  // serving worker
	Task    string       `json:"task"`    // execution task
	Run     string       `json:"run"`     // unique execution fence
	Status  HelperStatus `json:"status"`  // terminal status for settlement
	Summary string       `json:"summary"` // retained result
	Notice  string       `json:"notice"`  // rendered requester notice
}

// ownedOutcome distinguishes a current change from a replay or stale execution.
type ownedOutcome string

const (
	ownedApplied ownedOutcome = "applied" // current execution changed the request
	ownedReplay  ownedOutcome = "replay"  // the final notice was already published
	ownedStale   ownedOutcome = "stale"   // a different execution owns the request
)

// helperOwned changes one execution-owned request under the shared bus lock.
func (b *Bus) helperOwned(_ ownedCommand) (ownedOutcome, *CmdHelperOwnedError) {
	return ownedStale, nil
}
