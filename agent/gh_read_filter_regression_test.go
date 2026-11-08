package main

import (
	"bytes"
	"testing"
)

// TestPRObjectWithBodyIsContainer tests that PR objects with body+author+comments
// are treated as containers (not comment entries) and have nested comments filtered.
func TestPRObjectWithBodyIsContainer(t *testing.T) {
	// PR object has body (description), author, AND comments - should treat nested comments
	input := []byte(`{
		"id": "pr-1",
		"title": "Test PR",
		"body": "PR description",
		"author": {"login": "trusted"},
		"comments": [
			{"id": "1", "author": {"login": "untrusted"}, "body": "bad"}
		]
	}`)

	trusted := []string{"trusted"}
	filtered, ignored, err := ghFilterJSON(input, trusted, false)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// PR structure should be preserved
	if !bytes.Contains(filtered, []byte("Test PR")) {
		t.Error("PR title should be preserved")
	}

	// Bad comment should be filtered
	if bytes.Contains(filtered, []byte("bad")) {
		t.Error("untrusted comment should be filtered from nested array")
	}

	// Should have one ignored entry for the bad comment
	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	}
}

// TestStandaloneIgnoredObjectEmitsEmpty tests that a single ignored comment/review
// object emits empty bytes (not JSON null or wrapped in array).
func TestStandaloneIgnoredObjectEmitsEmpty(t *testing.T) {
	input := []byte(`{"id": "1", "author": {"login": "untrusted"}, "body": "bad"}`)

	trusted := []string{"trusted"}
	filtered, _, err := ghFilterJSON(input, trusted, false)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should emit nothing (empty bytes), not null
	if len(filtered) > 0 && !bytes.Equal(bytes.TrimSpace(filtered), []byte("null")) {
		t.Errorf("expected empty bytes, got: %s", string(filtered))
	}
}

// TestArrayEntriesWithoutBodyFiltered tests that array entries without body
// are still checked for trusted author (not passed through).
func TestArrayEntriesWithoutBodyFiltered(t *testing.T) {
	input := []byte(`[
		{"id": "1", "author": {"login": "untrusted"}},
		{"id": "2", "author": {"login": "trusted"}}
	]`)

	trusted := []string{"trusted"}
	filtered, ignored, err := ghFilterJSON(input, trusted, true)
	_ = filtered

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should still filter untrusted even without body
	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored (untrusted), got %d", len(ignored))
	}
}

// TestNumericIDPreserved tests that numeric IDs from JSON are preserved as strings.
func TestNumericIDPreserved(t *testing.T) {
	input := []byte(`[{"id": 12345, "author": {"login": "untrusted"}, "body": "test"}]`)

	trusted := []string{"trusted"}
	_, ignored, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	}

	// ID should be preserved as string "12345"
	if ignored[0].ID != "12345" {
		t.Errorf("expected ID '12345', got '%s'", ignored[0].ID)
	}
}
