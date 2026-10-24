package main

// cancel stops unstarted jobs immediately and requests a durable node stop after the first accepted start.
func (r *leadRegistry) cancel(id string, now Timestamp) (Job, *LeadCancelError) {
	r.mu.Lock()
	defer r.mu.Unlock()
	job, ok := r.jobs[id]
	if !ok {
		return Job{}, opErr(LeadCancelNoSuchJob, "no job %q", id)
	}
	switch job.State {
	case JobQueued, JobAssigned:
		job.State = JobStopped
		job.History = append(job.History, JobTransition{State: JobStopped, At: atOrAfterLast(job, now)})
	case JobRunning, JobWaiting:
		// The assigned node owns the process; the lead cannot claim it stopped before acknowledgement.
		job.StopRequested = true
	case JobDone, JobFailed, JobStopped:
		// Completion or an earlier cancellation already determined the terminal result.
	}
	return cloneJob(*job), nil
}
