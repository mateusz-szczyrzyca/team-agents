package main

// jobLeadTime preserves history ordering when receipt time was captured before a competing write or a clock adjustment.
// The caller holds the registry lock; node-owned report timestamps are validated separately.
func jobLeadTime(job *Job, now Timestamp) Timestamp {
	if len(job.History) > 0 {
		last := job.History[len(job.History)-1].At
		if now.Before(last.Time) {
			return last
		}
	}
	return now
}
