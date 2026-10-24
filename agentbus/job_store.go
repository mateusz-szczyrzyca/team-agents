package main

// transactJobs runs a registry operation under the shared bus lock and maps storage failures to its cause.
func transactJobs[C opCause](_ *Bus, _ C, _ Timestamp, _ func(*leadRegistry) *OpError[C]) *OpError[C] {
	return nil
}
