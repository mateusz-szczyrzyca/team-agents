package main

import (
	"context"
	"strings"
	"testing"
)

// TestGHDispatchViewJSONBodyOnly verifies that pr/issue view --json body
// responses without comments/reviews arrays are preserved.
func TestGHDispatchViewJSONBodyOnly(t *testing.T) {
	cases := []struct {
		name      string
		fields    string // --json fields to request
		response  string // simulated gh response
		wantEmpty bool   // should response be filtered to empty
	}{
		{"pr_view_json_body_only",
			"body,title,state",
			`{"body":"PR description","title":"PR title","state":"open"}`,
			false},
		{"issue_view_json_body",
			"body",
			`{"body":"Issue description"}`,
			false},
		{"pr_view_json_with_comments",
			"body,comments",
			`{"body":"PR","comments":[{"author":null,"body":"untrusted"}]}`,
			false}, // preserves structure but comments filtered by filterJSON
	}

	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			// Simulate pr/issue view --json with specific fields
			params := GHDispatchParams{
				Args:   []string{"pr", "view", "--json", c.fields},
				Stdout: &strings.Builder{},
				Stderr: &strings.Builder{},
				Env:    []string{"AGENT_GITHUB_TRUSTED_RESOLVED=alice"},
			}

			// Simulate handleView's logic: only filter if comment fields are present
			hasCommentFields := strings.Contains(c.fields, "comments") ||
				strings.Contains(c.fields, "reviews") ||
				strings.Contains(c.fields, "latestReviews")
			result, err := params.outputJSON(context.Background(), []byte(c.response), "", hasCommentFields)

			if err != nil {
				t.Errorf("outputJSON failed: %v", err)
			}
			if result != 0 {
				t.Errorf("expected exit 0, got %d", result)
			}

			// Check that body-only responses aren't completely filtered away
			output := params.Stdout.(*strings.Builder).String()
			isEmpty := len(strings.TrimSpace(output)) == 0
			if c.wantEmpty && !isEmpty {
				t.Errorf("expected filtered response to be empty, got: %s", output)
			} else if !c.wantEmpty && isEmpty {
				t.Errorf("expected response to be preserved, got empty output")
			}
		})
	}
}
