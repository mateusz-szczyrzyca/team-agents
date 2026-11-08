package main

import (
	"bytes"
	"testing"
)

// TestFilterJSONStandaloneReviewWithState verifies standalone reviews with state field are trust-checked.
func TestFilterJSONStandaloneReviewWithState(t *testing.T) {
	cases := []struct {
		name    string
		input   string
		trusted []string
		isEmpty bool
	}{
		{
			"standalone_review_approved_by_untrusted_is_filtered",
			`{"id":"1","state":"APPROVED","body":"untrusted","user":{"login":"outsider"}}`,
			[]string{},
			true,
		},
		{
			"standalone_review_approved_by_trusted_is_preserved",
			`{"id":"1","state":"APPROVED","body":"looks good","user":{"login":"alice"}}`,
			[]string{"alice"},
			false,
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			filtered, ignored, err := ghFilterJSON([]byte(c.input), c.trusted, true)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			isEmpty := len(bytes.TrimSpace(filtered)) == 0
			if isEmpty != c.isEmpty {
				t.Errorf("isEmpty: got %v, want %v (output: %s)", isEmpty, c.isEmpty, filtered)
			}
			if c.isEmpty && len(ignored) == 0 {
				t.Errorf("expected review to be ignored")
			}
		})
	}
}
