package main

import (
	"encoding/json"
)

// ownedCommand identifies the execution allowed to change a request or deliver its final notice.
type ownedCommand struct {
	ID      int64        `json:"id"`      // request number
	Action  ownedAction  `json:"action"`  // start, retry or settle
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

// ownedAction is a daemon-only transition of a fenced worker request.
type ownedAction string

const (
	ownedStart  ownedAction = "start"  // bind a newly claimed request to a fresh run
	ownedRetry  ownedAction = "retry"  // return a wrong result for another try
	ownedSettle ownedAction = "settle" // publish the final receipt and notice
)

// valid checks the complete command before any shared state is changed.
func (c ownedCommand) valid() bool {
	if c.ID <= 0 || c.Run == "" || c.Worker == "" || c.Task == "" {
		return false
	}
	switch c.Action {
	case ownedStart, ownedRetry:
		return true
	case ownedSettle:
		return (c.Status == HelperDone || c.Status == HelperFailed || c.Status == HelperUnclear) && c.Notice != ""
	}
	return false
}

// helperOwned serializes ownership and settlement with all other bus operations.
func (b *Bus) helperOwned(c ownedCommand) (ownedOutcome, *CmdHelperOwnedError) {
	if !c.valid() {
		return ownedStale, opErr(CmdHelperOwnedInvalidArgument, "invalid owned worker request transition")
	}
	result := ownedStale
	err := transact(b, CmdHelperOwnedStorage, func(st *State) *CmdHelperOwnedError {
		h := st.Helpers[helperKey(c.ID)]
		if h == nil {
			return opErr(CmdHelperOwnedNoSuchRequest, "no worker request H%d", c.ID)
		}
		if c.Action == ownedStart {
			if h.Status == HelperStarting && h.Worker == c.Worker && h.AttemptRun != c.Run {
				h.AttemptRun, h.HelperTask, h.Status = c.Run, c.Task, HelperRunning
				result = ownedApplied
			} else if h.Status == HelperRunning && h.Worker == c.Worker && h.HelperTask == c.Task && h.AttemptRun == c.Run {
				result = ownedReplay
			}
			return nil
		}
		if h.AttemptRun != c.Run || h.Worker != c.Worker || h.HelperTask != c.Task {
			return nil
		}
		if h.Status == HelperDone || h.Status == HelperFailed || h.Status == HelperUnclear {
			if c.Action == ownedSettle {
				result = ownedReplay
			}
			return nil
		}
		if h.Status != HelperRunning {
			return nil
		}
		if c.Action == ownedRetry {
			h.Status = HelperQueued
			result = ownedApplied
			return nil
		}
		// The receipt is the commit point. A lost state save is repaired from it on the next load.
		ev := EventScript{Notice: NoticeHelperResult, Body: c.Notice, Fields: map[string]string{
			"owned_request": helperKey(c.ID), "run": c.Run, "worker": c.Worker,
			"task": c.Task, "status": string(c.Status), "summary": c.Summary,
		}}
		if _, serr := b.postTo(st, binding{task: h.FromTask}, "agent", h.From, ev); serr != nil {
			return fromStorage(CmdHelperOwnedStorage, serr)
		}
		h.Status, h.Summary = c.Status, c.Summary
		result = ownedApplied
		return nil
	})
	return result, err
}

// restoreOwnedSettlements repairs a notice appended just before an interrupted state replacement.
// Only opted-in requests need a receipt scan; legacy requests retain their previous loading behavior.
func (b *Bus) restoreOwnedSettlements(st *State) *StorageError {
	needed := false
	for _, h := range st.Helpers {
		if h.AttemptRun != "" {
			needed = true
			break
		}
	}
	if !needed {
		return nil
	}
	messages, serr := b.messages()
	if serr != nil {
		return serr
	}
	for _, m := range messages {
		if m.ID >= st.NextID {
			st.NextID = m.ID + 1
		}
		if m.Kind != KindScript || m.From != "agent" {
			continue
		}
		var ev EventScript
		if err := json.Unmarshal(m.Data, &ev); err != nil {
			return storageErr(err, "cannot read owned settlement receipt %d", m.ID)
		}
		if ev.Notice != NoticeHelperResult || ev.Fields["owned_request"] == "" {
			continue
		}
		h := st.Helpers[ev.Fields["owned_request"]]
		if h == nil || h.AttemptRun != ev.Fields["run"] || h.HelperTask != ev.Fields["task"] || h.Worker != ev.Fields["worker"] {
			continue
		}
		status, verr := parseEnum[HelperStatus](ev.Fields["status"], "owned settlement status")
		if verr != nil {
			return storageErr(verr, "cannot read owned settlement receipt %d", m.ID)
		}
		if status != HelperDone && status != HelperFailed && status != HelperUnclear {
			return storageErr(nil, "nonterminal owned settlement receipt %d", m.ID)
		}
		h.Status, h.Summary = status, ev.Fields["summary"]
	}
	return nil
}

// cmdHelperOwned is the daemon's single atomic ownership/settlement interface.
func cmdHelperOwned(args []string) {
	fs := newFlagSet("helper-owned")
	dir := fs.String("dir", defaultDir(), "state directory")
	input := fs.String("input", "", "owned worker request transition JSON")
	parseFlags(fs, args)
	var command ownedCommand
	if err := json.Unmarshal([]byte(*input), &command); err != nil {
		exitOn(opErr(CmdHelperOwnedInvalidArgument, "invalid owned transition: %v", err).asError())
		return
	}
	result, err := (&Bus{dir: *dir}).helperOwned(command)
	exitOn(err.asError())
	exitOn(printJSON(result).asError())
}
