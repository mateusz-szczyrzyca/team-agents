package main

import (
	"bytes"
	"testing"
)

// TestFilterJSONStandaloneNullAuthor verifies corrected behavior: standalone entries with
// null author are filtered, but entries with missing author field are preserved
// (they're likely PR/issue responses from gh view commands, not comments).
func TestFilterJSONStandaloneNullAuthor(t *testing.T) {
	cases := []struct {
		name              string
		input             string
		trusted           []string
		wantFiltered      bool // true if should be filtered to empty
		wantIgnoredLength int
	}{
		{"standalone_comment_with_null_author_is_filtered",
			`{"id":"99","author":null,"body":"malicious"}`,
			[]string{}, true, 1},
		{"response_with_missing_author_is_preserved",
			`{"id":"100","body":"untrusted"}`,
			[]string{}, false, 0},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			filtered, ignored, err := ghFilterJSON([]byte(c.input), c.trusted, true)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			isEmpty := len(bytes.TrimSpace(filtered)) == 0
			if isEmpty != c.wantFiltered {
				t.Errorf("filtered: got %v, want %v (output: %s)", isEmpty, c.wantFiltered, filtered)
			}
			if len(ignored) != c.wantIgnoredLength {
				t.Errorf("ignored count: got %d, want %d", len(ignored), c.wantIgnoredLength)
			}
		})
	}
}
