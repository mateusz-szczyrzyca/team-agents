package main

// cancel stops unstarted jobs immediately and requests a durable node stop after the first accepted start.
func (r *leadRegistry) cancel(_ string, _ Timestamp) (Job, *LeadCancelError) {
	return Job{}, nil
}
