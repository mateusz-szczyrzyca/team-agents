package main

// remoteHelperResult carries the bounded terminal outcome back to the requesting local bus.
type remoteHelperResult struct {
	State  JobState // actual terminal job outcome, including stopped
	Node   string   // executing instance, empty for pre-submission failures
	Branch string   // pushed result branch, empty when no checkout was made
	PRURL  string   // optional pull request link
	Result string   // head of the final message or actionable failure reason
}

// takeLocalHelper claims the oldest eligible request; remote-bound requests stay with the daemon.
func takeLocalHelper(_ *State, _ string, _ []string) *Helper { return nil }

// remoteHelpers returns open requests that require another node or already have a frozen job.
func remoteHelpers(_ *Bus, _ []string) ([]*Helper, *HelperRemoteListError) { return nil, nil }

// prepareRemoteHelper verifies the pushed snapshot and persists a retry-stable submission outside network calls.
func prepareRemoteHelper(_ *Bus, _ int64, _ string) (JobRequest, *HelperRemotePrepareError) {
	return JobRequest{}, nil
}

// finishRemoteHelper records terminal state and delivers the ordinary helper-result notice.
func finishRemoteHelper(_ *Bus, _ int64, _ remoteHelperResult) *HelperRemoteFinishError { return nil }
