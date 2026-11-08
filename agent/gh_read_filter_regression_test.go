package main

import (
	"bytes"
	"testing"
)

// TestFilterJSONStandaloneNullAuthor verifies standalone comments with null/missing authors are filtered.
func TestFilterJSONStandaloneNullAuthor(t *testing.T) {
	cases := []struct {
		name    string
		input   string
		trusted []string
		want    string
	}{
		{"standalone_comment_with_null_author_is_filtered",
			`{"id":"99","author":null,"body":"malicious"}`,
			[]string{}, ``},
		{"standalone_comment_with_missing_author_is_filtered",
			`{"id":"100","body":"untrusted"}`,
			[]string{}, ``},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			filtered, ignored, err := ghFilterJSON([]byte(c.input), c.trusted, true)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if !bytes.Equal(bytes.TrimSpace(filtered), []byte(c.want)) {
				t.Errorf("output: got %s, want %s", filtered, c.want)
			}
			if len(ignored) == 0 {
				t.Errorf("expected comment to be ignored")
			}
		})
	}
}
