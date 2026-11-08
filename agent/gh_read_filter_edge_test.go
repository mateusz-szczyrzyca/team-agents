package main

import (
	"bytes"
	"testing"
)

// TestFilterJSONEdgeCases pins the H1054 edge fixes: an all-rejected nested
// array marshals as [], a null id falls back to databaseId, and every array
// map is trust-checked even when its author is missing or null.
func TestFilterJSONEdgeCases(t *testing.T) {
	cases := []struct {
		name    string
		input   string
		trusted []string
		want    string
		wantIDs []string
	}{
		{"container comments all rejected keep empty array",
			`{"comments":[{"id":"1","author":{"login":"bob"},"body":"x"}]}`,
			[]string{"alice"}, `{"comments":[]}`, []string{"1"}},
		{"null id falls back to databaseId",
			`[{"id":null,"databaseId":12345,"author":{"login":"bob"},"body":"x"}]`,
			[]string{"alice"}, `[]`, []string{"12345"}},
		{"missing or null author array maps rejected",
			`[{"body":"no author"},{"author":null,"body":"null author"},{"id":"3","author":{"login":"alice"},"body":"ok"}]`,
			[]string{"alice"}, `[{"author":{"login":"alice"},"body":"ok","id":"3"}]`, []string{"", ""}},
		{"standalone comment with null author is filtered",
			`{"id":"99","author":null,"body":"malicious"}`,
			[]string{}, ``, []string{"99"}},
		{"standalone comment with missing author is filtered",
			`{"id":"100","body":"untrusted"}`,
			[]string{}, ``, []string{"100"}},
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
			if len(ignored) != len(c.wantIDs) {
				t.Fatalf("ignored: got %d %+v, want %d", len(ignored), ignored, len(c.wantIDs))
			}
			for i, id := range c.wantIDs {
				if ignored[i].ID != id {
					t.Errorf("ignored[%d].ID: got %q, want %q", i, ignored[i].ID, id)
				}
			}
		})
	}
}
