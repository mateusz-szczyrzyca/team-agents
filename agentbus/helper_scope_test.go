package main

import (
	"encoding/json"
	"os"
	"testing"
)

// TestRemoteHelperScopeSurvivesFreeze verifies the discriminator through the serialized job boundary.
func TestRemoteHelperScopeSurvivesFreeze(t *testing.T) {
	b := newBus(t)
	remoteHelperFixture(t, b)
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) { return nil, nil })
	for range 2 {
		req, err := prepareRemoteHelper(b, 1, "lead")
		if err != nil {
			t.Fatal(err)
		}
		data, jerr := json.Marshal(req)
		if jerr != nil {
			t.Fatal(jerr)
		}
		var fields map[string]any // inspect the payload without requiring a new field to compile the RED test
		if jerr := json.Unmarshal(data, &fields); jerr != nil {
			t.Fatal(jerr)
		}
		if fields["Helper"] != true || fields["Requester"] != "member@lead" {
			t.Fatalf("delegation scope lost: %s", data)
		}
	}
}

// TestJobBeforeHelperScopeLoads preserves the ordinary worker meaning of old requests.
func TestJobBeforeHelperScopeLoads(t *testing.T) {
	data, err := os.ReadFile("testdata/job-before-helper-scope.json")
	if err != nil {
		t.Fatal(err)
	}
	var req JobRequest // the pre-change submission has no helper discriminator
	if err := json.Unmarshal(data, &req); err != nil {
		t.Fatal(err)
	}
	if _, err := validateJobRequest(req); err != nil {
		t.Fatal(err)
	}
	out, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]any // a false discriminator stays omitted for old peers
	if err := json.Unmarshal(out, &fields); err != nil {
		t.Fatal(err)
	}
	if _, exists := fields["Helper"]; exists {
		t.Fatal("old request acquired helper permissions")
	}
}
