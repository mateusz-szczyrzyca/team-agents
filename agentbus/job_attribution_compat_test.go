package main

import (
	"encoding/json"
	"testing"
)

// TestJobAttributionEmptyWireCompatibility preserves exact JSON comparisons for old frozen helper requests.
func TestJobAttributionEmptyWireCompatibility(t *testing.T) {
	data, err := json.Marshal(regRequest("legacy"))
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]json.RawMessage // optional empty fields must remain absent on the wire
	if err := json.Unmarshal(data, &fields); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"RepoURL", "RequesterTask"} {
		if _, exists := fields[key]; exists {
			t.Fatalf("empty %s changes old frozen request identity", key)
		}
	}
}
