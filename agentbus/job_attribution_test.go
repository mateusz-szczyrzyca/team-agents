package main

import (
	_ "embed"
	"errors"
	"os"
	"testing"
)

// TestJobAttributionValidation bounds optional link data before it can reach a view.
func TestJobAttributionValidation(t *testing.T) {
	for _, raw := range []string{"http://github.com/acme/repo", "https://evil.example/acme/repo", "https://github.com/acme/repo/tree/main", "https://github.com/acme/repo?x=1", "https://github.com/acme/repo#x", "https://github.com/acme/..", "https://github.com/acme/repo.git", "https://github.com/acme/repo/", "https://github.com/a@github.com/repo"} {
		t.Run(raw, func(t *testing.T) {
			req := regRequest("links")
			req.RepoURL = raw
			_, err := newLeadRegistry().submit(req, regAt(0))
			var failure *LeadSubmitError // exact operation cause must survive errors.As
			if !errors.As(err.asError(), &failure) || failure.Cause != LeadSubmitInvalidRepoURL {
				t.Fatalf("invalid repository link accepted: %v", err)
			}
		})
	}
	req := regRequest("task")
	req.RequesterTask = "../task"
	_, err := newLeadRegistry().submit(req, regAt(0))
	failsWith(t, LeadSubmitInvalidRequesterTask)("requester task path", err)
}

// TestJobAttributionRoundtrip keeps attribution immutable and present after restart.
func TestJobAttributionRoundtrip(t *testing.T) {
	b := newBus(t)
	req := regRequest("links")
	req.RepoURL = "https://github.com/acme/repo"
	req.RequesterTask = "task-member"
	storeRun(t, b, regAt(0), func(r *leadRegistry) { regSubmit(t, r, req, regAt(0)) })
	storeRun(t, b, regAt(1), func(r *leadRegistry) {
		got := r.jobs[req.ID].Request
		if got.RepoURL != req.RepoURL || got.RequesterTask != req.RequesterTask {
			t.Fatalf("lost attribution: %+v", got)
		}
		req.RequesterTask = "different-task"
		_, err := r.submit(req, regAt(1))
		failsWith(t, LeadSubmitJobConflict)("changed attribution", err)
	})
}

// jobsBeforeAttribution preserves the preceding on-disk format without filesystem input.
//
//go:embed testdata/jobs-before-attribution.json
var jobsBeforeAttribution []byte

// TestJobsBeforeAttributionLoads preserves the immediately preceding jobs format.
func TestJobsBeforeAttributionLoads(t *testing.T) {
	b := newBus(t)
	if err := os.WriteFile(b.path("jobs.json"), jobsBeforeAttribution, 0o600); err != nil {
		t.Fatal(err)
	}
	r, serr := b.loadJobs()
	if serr != nil {
		t.Fatal(serr)
	}
	if got := r.jobs["old-job"].Request; got.RepoURL != "" || got.RequesterTask != "" {
		t.Fatalf("legacy attribution: %+v", got)
	}
}

// TestRemoteHelperAttribution uses the origin and delegating task in a frozen request.
func TestRemoteHelperAttribution(t *testing.T) {
	b := newBus(t)
	h := remoteHelperFixture(t, b)
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) { return nil, nil })
	got, err := prepareRemoteHelper(b, h.ID, "node-a", "https://github.com/acme/repo")
	if err != nil {
		t.Fatal(err)
	}
	if got.RepoURL != "https://github.com/acme/repo" || got.RequesterTask != h.FromTask {
		t.Fatalf("missing helper attribution: %+v", got)
	}
}
