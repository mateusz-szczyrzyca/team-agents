package main

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"strings"
)

// remoteHelperResult carries the bounded terminal outcome back to the requesting local bus.
type remoteHelperResult struct {
	State  JobState // actual terminal job outcome, including stopped
	Node   string   // executing instance, empty for pre-submission failures
	Branch string   // pushed result branch, empty when no checkout was made
	PRURL  string   // optional pull request link
	Result string   // head of the final message or actionable failure reason
}

// takeLocalHelper claims the oldest eligible request; remote-bound requests stay with the daemon.
func takeLocalHelper(st *State, worker string, capabilities []string) *Helper {
	for _, h := range sortedHelpers(st) {
		if h.Status == HelperQueued && h.RemoteJob == nil && jobHasCapabilities(capabilities, h.Requires) {
			h.Status, h.Worker = HelperStarting, worker
			return h
		}
	}
	return nil
}

// remoteHelpers returns open requests that require another node or already have a frozen job.
func remoteHelpers(b *Bus, capabilities []string) ([]*Helper, *HelperRemoteListError) {
	if _, err := canonicalJobCapabilities(capabilities); err != nil {
		return nil, opErr(HelperRemoteListInvalidCapabilities, "%v", err)
	}
	out := []*Helper{} // encode an empty queue as an array for shell consumers
	err := transact(b, HelperRemoteListStorage, func(st *State) *HelperRemoteListError {
		for _, h := range sortedHelpers(st) {
			if h.Status.Open() && (h.RemoteJob != nil || h.Status == HelperQueued && !jobHasCapabilities(capabilities, h.Requires)) {
				out = append(out, h)
			}
		}
		return nil
	})
	return out, err
}

// prepareRemoteHelper verifies the pushed snapshot and persists a retry-stable submission outside network calls.
func prepareRemoteHelper(b *Bus, id int64, instance string) (JobRequest, *HelperRemotePrepareError) {
	var h Helper // owned snapshot; Git never runs with bus.lock held
	if err := transact(b, HelperRemotePrepareStorage, func(st *State) *HelperRemotePrepareError {
		current := st.Helpers[helperKey(id)]
		if current == nil {
			return opErr(HelperRemotePrepareNoSuchRequest, "no worker request H%d", id)
		}
		h = *current
		return nil
	}); err != nil {
		return JobRequest{}, err
	}
	if !validJobIdentifier(instance) || !h.Status.Open() || h.RemoteJob == nil && h.Status != HelperQueued {
		return JobRequest{}, opErr(HelperRemotePrepareInvalidRequest, "helper H%d is not a queued remote request or instance is invalid", id)
	}
	if h.RemoteJob != nil {
		return *h.RemoteJob, nil
	}
	// Bind the idempotency key to the local request and creation, not a mutable branch tip.
	key := sha256.Sum256([]byte(fmt.Sprintf("%s:%d:%s:%s", instance, id, h.Created, h.FromTask)))
	req := JobRequest{ID: fmt.Sprintf("delegate-%x", key[:16]), Repository: h.Repo, BaseRef: h.Branch, BaseSHA: h.BaseSHA, Requires: h.Requires, Selector: string(TierWorker), Task: h.Request, Requester: h.From + "@" + instance}
	if h.Context != "" {
		req.Task += "\n\nContext:\n" + h.Context
	}
	validated, verr := validateJobRequest(req)
	if verr != nil || len(h.Requires) == 0 {
		return JobRequest{}, opErr(HelperRemotePrepareInvalidRequest, "cannot delegate H%d: invalid snapshot or job data (%v)", id, verr)
	}
	req = validated
	if err := verifyRemoteHelperPush(h); err != nil {
		return JobRequest{}, err
	}
	err := transact(b, HelperRemotePrepareStorage, func(st *State) *HelperRemotePrepareError {
		current := st.Helpers[helperKey(id)]
		if current == nil {
			return opErr(HelperRemotePrepareNoSuchRequest, "no worker request H%d", id)
		}
		if current.RemoteJob != nil {
			req = *current.RemoteJob
			return nil
		}
		if current.Status != HelperQueued || current.BaseSHA != h.BaseSHA {
			return opErr(HelperRemotePrepareInvalidRequest, "helper H%d changed during push verification", id)
		}
		current.RemoteJob = &req
		return nil
	})
	return req, err
}

// verifyRemoteHelperPush fetches only the requester's branch into an isolated ref, checking real remote ancestry.
func verifyRemoteHelperPush(h Helper) *HelperRemotePrepareError {
	var nonce [16]byte // private refs avoid reliance on FETCH_HEAD and shared tracking-ref races
	if _, err := rand.Read(nonce[:]); err != nil {
		return fromStorage(HelperRemotePrepareStorage, storageErr(err, "cannot allocate push verification ref"))
	}
	ref := fmt.Sprintf("refs/team-agents/helper-push/%x", nonce)
	defer func() {
		if _, err := callNodeGit(h.Worktree, "update-ref", "-d", ref); err != nil {
			warnSideEffect("remove helper push ref", err, "ref", ref)
		}
	}()
	branch := "refs/heads/" + strings.TrimPrefix(h.Branch, "refs/heads/")
	if _, err := callNodeGit(h.Worktree, "fetch", "--no-tags", "--no-recurse-submodules", "--", "origin", branch+":"+ref); err != nil {
		return &HelperRemotePrepareError{Cause: HelperRemotePrepareNotPushed, Message: fmt.Sprintf("cannot verify %s on origin/%s; push the branch before delegating: %v", h.BaseSHA, h.Branch, err), Err: err}
	}
	if _, err := callNodeGit(h.Worktree, "merge-base", "--is-ancestor", h.BaseSHA, ref); err != nil {
		return &HelperRemotePrepareError{Cause: HelperRemotePrepareNotPushed, Message: fmt.Sprintf("commit %s is not reachable from origin/%s; push it before delegating: %v", h.BaseSHA, h.Branch, err), Err: err}
	}
	return nil
}

// finishRemoteHelper records terminal state and delivers the ordinary helper-result notice.
// The notice itself identifies delivery if posting succeeds but saving helper state fails.
func finishRemoteHelper(b *Bus, id int64, result remoteHelperResult) *HelperRemoteFinishError {
	if result.State != JobDone && result.State != JobFailed && result.State != JobStopped {
		return opErr(HelperRemoteFinishInvalidState, "remote result must be done, failed or stopped")
	}
	return transact(b, HelperRemoteFinishStorage, func(st *State) *HelperRemoteFinishError {
		h := st.Helpers[helperKey(id)]
		if h == nil {
			return opErr(HelperRemoteFinishNoSuchRequest, "no worker request H%d", id)
		}
		if !h.Status.Open() {
			return nil
		}
		summary := fmt.Sprintf("RESULT H%d [%s]: node %s; branch %s", id, result.State, result.Node, result.Branch)
		if result.PRURL != "" {
			summary += "; PR " + result.PRURL
		}
		summary += "\n" + trunc(result.Result, 3500)
		msgs, serr := b.messages()
		if serr != nil {
			return fromStorage(HelperRemoteFinishStorage, serr)
		}
		delivered := false // suppress a repeated local notice after a failed state save
		for _, msg := range msgs {
			if msg.ID >= st.NextID {
				st.NextID = msg.ID + 1
			}
			if msg.Kind != KindScript || msg.To != h.From {
				continue
			}
			var event EventScript // only our ordinary script notice establishes prior delivery
			if err := json.Unmarshal(msg.Data, &event); err != nil {
				return fromStorage(HelperRemoteFinishStorage, storageErr(err, "cannot decode helper result notice"))
			}
			if event.Notice == NoticeHelperResult && event.Fields["request"] == fmt.Sprintf("H%d", id) && event.Fields["requesting_task"] == h.FromTask {
				delivered = true
				summary = event.Body
				result.State = JobState(event.Fields["status"])
				break
			}
		}
		if !delivered {
			_, serr = b.post(st, "agentbus", h.From, EventScript{Notice: NoticeHelperResult, Body: summary, Fields: map[string]string{"request": fmt.Sprintf("H%d", id), "requesting_task": h.FromTask, "status": string(result.State), "node": result.Node, "branch": result.Branch}})
			if serr != nil {
				return fromStorage(HelperRemoteFinishStorage, serr)
			}
		}
		h.Status, h.Summary = HelperFailed, summary
		if result.State == JobDone {
			h.Status = HelperDone
		}
		return nil
	})
}

// helperCapabilities decodes and validates the local daemon's explicit declaration.
func helperCapabilities(raw string) ([]string, *CapsParseError) {
	var caps []string // absence of capabilities means ordinary requests alone are eligible
	if err := json.Unmarshal([]byte(raw), &caps); err != nil {
		return nil, opErr(CapsParseMalformed, "capabilities must be a JSON array of strings: %v", err)
	}
	return canonicalJobCapabilities(caps)
}

// cmdHelperRemoteList is the local daemon's durable outgoing helper queue adapter.
func cmdHelperRemoteList(args []string) {
	fs := newFlagSet("helper-remote-list")
	dir := fs.String("dir", defaultDir(), "state directory")
	caps := fs.String("capabilities", "[]", "local capabilities JSON array")
	parseFlags(fs, args)
	capabilities, err := helperCapabilities(*caps)
	if err != nil {
		exitOn(opErr(HelperRemoteListInvalidCapabilities, "%v", err).asError())
		return
	}
	list, lerr := remoteHelpers(&Bus{dir: *dir}, capabilities)
	if lerr != nil {
		exitOn(lerr.asError())
		return
	}
	exitOn(fromStorage(HelperRemoteListStorage, printJSON(list)).asError())
}

// cmdHelperRemotePrepare freezes an origin-verified submission before the daemon sends it.
func cmdHelperRemotePrepare(args []string) {
	fs := newFlagSet("helper-remote-prepare")
	dir := fs.String("dir", defaultDir(), "state directory")
	id := fs.Int64("id", 0, "helper number")
	instance := fs.String("instance", "", "requesting instance name")
	parseFlags(fs, args)
	req, err := prepareRemoteHelper(&Bus{dir: *dir}, *id, *instance)
	if err != nil {
		err.Message = err.CauseName() + ": " + err.Message
		exitOn(err.asError())
		return
	}
	exitOn(fromStorage(HelperRemotePrepareStorage, printJSON(req)).asError())
}

// cmdHelperRemoteFinish delivers a terminal job result to its local requester.
func cmdHelperRemoteFinish(args []string) {
	fs := newFlagSet("helper-remote-finish")
	dir := fs.String("dir", defaultDir(), "state directory")
	id := fs.Int64("id", 0, "helper number")
	state := fs.String("state", "", "terminal job state")
	node := fs.String("node", "", "executing node")
	branch := fs.String("branch", "", "pushed result branch")
	pr := fs.String("pr", "", "optional pull request")
	result := fs.String("result", "", "head of final result")
	parseFlags(fs, args)
	exitOn(finishRemoteHelper(&Bus{dir: *dir}, *id, remoteHelperResult{State: JobState(*state), Node: *node, Branch: *branch, PRURL: *pr, Result: *result}).asError())
}
