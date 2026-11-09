package main

import (
	"encoding/json"
	"testing"
)

// TestFilterJSONPRIssueViewResponses verifies B44 fix: gh pr/issue view responses
// with body but no author field are preserved.
func TestFilterJSONPRIssueViewResponses(t *testing.T) {
	cases := []struct {
		name      string
		input     string
		trusted   []string
		wantEmpty bool
	}{
		{"pr_view_response_with_body_no_author_preserved",
			`{"number":1,"title":"PR title","body":"PR description","state":"open"}`,
			[]string{}, false},
		{"issue_view_response_with_body_no_author_preserved",
			`{"number":2,"title":"Issue title","body":"Issue description","state":"open"}`,
			[]string{}, false},
		{"comment_with_null_author_is_filtered",
			`{"id":"99","author":null,"body":"untrusted comment"}`,
			[]string{}, true},
		{"comment_with_author_object_and_body_is_checked",
			`{"id":"100","author":{"login":"alice"},"body":"trusted comment"}`,
			[]string{"alice"}, false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			filtered, _, err := ghFilterJSON([]byte(c.input), c.trusted, true)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			isEmpty := len(filtered) == 0
			if isEmpty != c.wantEmpty {
				t.Errorf("empty: got %v, want %v (output: %s)", isEmpty, c.wantEmpty, filtered)
			}
			if !isEmpty {
				var output interface{}
				if err := json.Unmarshal(filtered, &output); err != nil {
					t.Fatalf("failed to parse output: %v", err)
				}
			}
		})
	}
}
