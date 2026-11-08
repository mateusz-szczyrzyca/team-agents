package main

import (
	"bytes"
	"testing"
)

// TestFilterJSONSlurpedPages verifies that nested arrays produced by --slurp
// (array of pages where each page is an array of entries) are recursively filtered,
// removing untrusted comments and logging ignored entries.
func TestFilterJSONSlurpedPages(t *testing.T) {
	input := `[[{"id":"1","author":{"login":"untrusted"},"body":"bad comment"},{"id":"2","author":{"login":"alice"},"body":"good comment"}],[{"id":"3","author":{"login":"eve"},"body":"evil comment"}]]`
	trusted := []string{"alice"}
	want := `[[{"author":{"login":"alice"},"body":"good comment","id":"2"}],[]]`

	filtered, ignored, err := ghFilterJSON([]byte(input), trusted, true)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !bytes.Equal(bytes.TrimSpace(filtered), []byte(want)) {
		t.Fatalf("got %s, want %s", filtered, want)
	}
	if len(ignored) != 2 {
		t.Fatalf("ignored length: got %d, want 2 (%+v)", len(ignored), ignored)
	}
	if ignored[0].ID != "1" || ignored[1].ID != "3" {
		t.Fatalf("ignored IDs: got [%s, %s], want [1, 3]", ignored[0].ID, ignored[1].ID)
	}
}
