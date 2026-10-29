package main

import (
	"strings"
	"sync"
	"testing"
)

// A remote member's pause on its node (a provider's limit, the network) reaches the lead once, with its reason —
// its latest activity line —, and its return to work once.
func TestRemoteMemberPauseReachesTheLeadOnceWithItsReason(t *testing.T) {
	_, a, _, _ := remoteTeam(t)
	var mu sync.Mutex
	got := []string{}
	old := remoteMemberFn
	remoteMemberFn = func(_ *Team, _, status, _, note string) {
		mu.Lock()
		defer mu.Unlock()
		got = append(got, status+" "+note)
	}
	t.Cleanup(func() { remoteMemberFn = old })
	reason := "■ paused: the provider's quota or usage limit is exhausted — resumes at 01.11 00:01"
	for _, act := range []JobActivity{
		{ID: "job-m", Agent: "solo", Stage: "running", Lines: []string{"working"}},
		{ID: "job-m", Agent: "solo", Stage: "paused", Lines: []string{"working", reason}},
		{ID: "job-m", Agent: "solo", Stage: "paused", Lines: []string{"working", reason}}, // the same pause: once
		{ID: "job-m", Agent: "solo", Stage: "running", Lines: []string{"back at work"}},
	} {
		if _, err := a.memberActivity("node2", []JobActivity{act}); err != nil {
			t.Fatal(err)
		}
	}
	want := []string{"paused " + reason, "running "}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("changes %q, want %q", got, want)
	}
}
