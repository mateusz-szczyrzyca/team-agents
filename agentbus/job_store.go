package main

import (
	"encoding/json"
	"errors"
	"os"
	"time"
)

// jobSnapshot is the first persisted registry format; ordinary bus state remains independent.
type jobSnapshot struct {
	Version        int                      // format discriminator, currently 1; unknown formats fail closed
	Jobs           []storedJob              // jobs in submission order, including equal-time submissions
	Nodes          map[string]storedJobNode // trusted declarations and receipt times retained across restarts
	NextSequence   uint64                   // high-water mark, retained even after all jobs expire
	NextAssignment uint64                   // assignment high-water mark; old evidence is never reused
}

// jobTime preserves the instant and subsecond precision without changing the legacy bus timestamp format.
type jobTime struct {
	Timestamp // ordinary in-memory representation, with only JSON encoding overridden
}

// MarshalJSON implements json.Marshaler with explicit UTC and full precision.
func (t jobTime) MarshalJSON() ([]byte, error) {
	if t.IsZero() {
		return json.Marshal("")
	}
	return json.Marshal(t.UTC().Format(time.RFC3339Nano))
}

// storedTransition shadows only the timestamp representation of the in-memory history.
type storedTransition struct {
	State JobState // state entered at this transition
	At    jobTime  // precise transition instant, independent of the lead's timezone
}

// storedJob keeps new ordinary job fields while shadowing timezone-sensitive history timestamps.
type storedJob struct {
	Job                        // the registry's job fields, except the shadowed history
	History []storedTransition // history with lossless timestamps
}

// storedJobNode shadows the receipt timestamp without changing heartbeat fields.
type storedJobNode struct {
	jobNode          // the declaration retained by the registry
	LastSeen jobTime // precise lead receipt instant
}

// transactJobs runs a registry operation under the shared bus lock and maps storage failures to its cause.
// The registry belongs only to this callback; rejected operations never replace jobs.json.
func transactJobs[C opCause](b *Bus, storage C, now Timestamp, fn func(*leadRegistry) *OpError[C]) *OpError[C] {
	var failure *OpError[C]
	serr := b.withLock(func() *StorageError {
		r, err := b.loadJobs()
		if err != nil {
			return err
		}
		r.pruneJobs(now)
		failure = fn(r)
		if failure != nil {
			return nil
		}
		return b.saveJobs(r)
	})
	if serr != nil {
		return fromStorage(storage, serr)
	}
	return failure
}

// loadJobs reads an entire snapshot while the caller holds bus.lock; missing storage is the only empty-file case.
func (b *Bus) loadJobs() (*leadRegistry, *StorageError) {
	data, err := os.ReadFile(b.path("jobs.json"))
	if errors.Is(err, os.ErrNotExist) {
		return newLeadRegistry(), nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read jobs.json")
	}
	var snapshot jobSnapshot
	if err := json.Unmarshal(data, &snapshot); err != nil {
		return nil, storageErr(err, "cannot decode jobs.json")
	}
	if snapshot.Version != 1 {
		return nil, storageErr(ErrStorage, "unsupported jobs.json format %d", snapshot.Version)
	}
	r := newLeadRegistry()
	r.nextSequence, r.nextAssignment = snapshot.NextSequence, snapshot.NextAssignment
	var previous uint64
	for _, stored := range snapshot.Jobs {
		job := stored.Job
		for _, transition := range stored.History {
			job.History = append(job.History, JobTransition{State: transition.State, At: transition.At.Timestamp})
		}
		if _, err := validateJobRequest(job.Request); err != nil {
			return nil, storageErr(err, "invalid stored job request")
		}
		if r.jobs[job.Request.ID] != nil || job.Seq <= previous || job.Seq > r.nextSequence || job.Assignment > r.nextAssignment ||
			len(job.History) == 0 || job.History[len(job.History)-1].State != job.State {
			return nil, storageErr(ErrStorage, "inconsistent stored job %q", job.Request.ID)
		}
		previous = job.Seq
		for i, transition := range job.History {
			if transition.At.IsZero() || i > 0 && transition.At.Before(job.History[i-1].At.Time) {
				return nil, storageErr(ErrStorage, "invalid stored job history %q", job.Request.ID)
			}
		}
		r.jobs[job.Request.ID] = &job
		r.order = append(r.order, job.Request.ID)
	}
	for name, stored := range snapshot.Nodes {
		node := stored.jobNode
		node.LastSeen = stored.LastSeen.Timestamp
		if name != node.Heartbeat.Node || node.LastSeen.IsZero() {
			return nil, storageErr(ErrStorage, "invalid stored node %q", name)
		}
		// Reuse admission validation without risking an assignment from the restored queue.
		if _, err := newLeadRegistry().heartbeat(node.Heartbeat, node.LastSeen); err != nil {
			return nil, storageErr(err, "invalid stored node %q", name)
		}
		r.nodes[name] = node
	}
	return r, nil
}

// saveJobs atomically replaces only the job snapshot; its caller holds bus.lock and owns r exclusively.
func (b *Bus) saveJobs(r *leadRegistry) *StorageError {
	snapshot := jobSnapshot{Version: 1, Nodes: make(map[string]storedJobNode), NextSequence: r.nextSequence, NextAssignment: r.nextAssignment}
	for name, node := range r.nodes {
		snapshot.Nodes[name] = storedJobNode{jobNode: node, LastSeen: jobTime{node.LastSeen}}
	}
	for _, id := range r.order {
		job := *r.jobs[id]
		stored := storedJob{Job: job}
		for _, transition := range job.History {
			stored.History = append(stored.History, storedTransition{State: transition.State, At: jobTime{transition.At}})
		}
		snapshot.Jobs = append(snapshot.Jobs, stored)
	}
	data, err := json.MarshalIndent(snapshot, "", "  ")
	if err != nil {
		return storageErr(err, "cannot encode jobs.json")
	}
	tmp := b.path("jobs.json.tmp")
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return storageErr(err, "cannot write %s", tmp)
	}
	if err := os.Rename(tmp, b.path("jobs.json")); err != nil {
		return storageErr(err, "cannot replace jobs.json")
	}
	return nil
}

// pruneJobs expires terminal outcomes from their last transition, leaving active jobs and counter evidence intact.
// The caller owns the registry exclusively; retention is applied before every persisted operation.
func (r *leadRegistry) pruneJobs(now Timestamp) {
	kept := r.order[:0]
	for _, id := range r.order {
		job := r.jobs[id]
		terminal := job.State == JobDone || job.State == JobFailed || job.State == JobStopped
		if terminal && now.Sub(job.History[len(job.History)-1].At.Time) >= 30*24*time.Hour {
			delete(r.jobs, id)
			continue
		}
		kept = append(kept, id)
	}
	r.order = kept
}
