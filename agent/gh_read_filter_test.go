package main

import (
	"bytes"
	"testing"
)

// TestFilterJSONBasicTrustedAuthor tests filtering keeps comments from trusted authors.
func TestFilterJSONBasicTrustedAuthor(t *testing.T) {
	input := []byte(`[
		{"id": "1", "author": {"login": "trusted-user"}, "body": "this is a comment"},
		{"id": "2", "author": {"login": "untrusted-user"}, "body": "another comment"}
	]`)

	trusted := []string{"trusted-user"}
	filtered, ignored, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should keep trusted-user's comment
	if !bytes.Contains(filtered, []byte("trusted-user")) {
		t.Error("trusted-user comment should be in filtered output")
	}

	// Should remove untrusted-user's comment
	if bytes.Contains(filtered, []byte("untrusted-user")) {
		t.Error("untrusted-user comment should not be in filtered output")
	}

	// Should have one ignored entry
	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	} else if ignored[0].Reason != ghIgnoreReasonUntrustedAuthor {
		t.Errorf("expected untrusted author, got %v", ignored[0].Reason)
	}
}

// TestFilterJSONMarkedIgnore tests filtering removes "ignore this mate" comments.
func TestFilterJSONMarkedIgnore(t *testing.T) {
	input := []byte(`[
		{"id": "1", "author": {"login": "trusted-user"}, "body": "ignore this mate - remove me"},
		{"id": "2", "author": {"login": "trusted-user"}, "body": "keep this comment"}
	]`)

	trusted := []string{"trusted-user"}
	filtered, ignored, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should remove marked comment
	if bytes.Contains(filtered, []byte("remove me")) {
		t.Error("marked comment should be removed")
	}

	// Should keep non-marked comment
	if !bytes.Contains(filtered, []byte("keep this")) {
		t.Error("normal comment should be kept")
	}

	// Should have one ignored entry with marked reason
	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	} else if ignored[0].Reason != ghIgnoreReasonMarkedIgnore {
		t.Errorf("expected marked ignore, got %v", ignored[0].Reason)
	}
}

// TestFilterJSONCaseInsensitiveIgnore tests "ignore this mate" is case-insensitive.
func TestFilterJSONCaseInsensitiveIgnore(t *testing.T) {
	input := []byte(`[
		{"id": "1", "author": {"login": "trusted"}, "body": "  IGNORE THIS MATE: remove"},
		{"id": "2", "author": {"login": "trusted"}, "body": "  Ignore This Mate\nmore text"}
	]`)

	trusted := []string{"trusted"}
	_, ignored, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should have both ignored
	if len(ignored) != 2 {
		t.Errorf("expected 2 ignored entries, got %d", len(ignored))
	}
}

// TestFilterJSONUntrustedVsMarkedPrecedence tests untrusted is checked before marked.
func TestFilterJSONUntrustedVsMarkedPrecedence(t *testing.T) {
	input := []byte(`[
		{"id": "1", "author": {"login": "untrusted"}, "body": "ignore this mate - should say untrusted"}
	]`)

	trusted := []string{"trusted-user"}
	_, ignored, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should be filtered for untrusted, not marked reason
	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	} else if ignored[0].Reason != ghIgnoreReasonUntrustedAuthor {
		t.Errorf("expected untrusted_author precedence, got %v", ignored[0].Reason)
	}
}

// TestFilterJSONNestedComments tests filtering nested comments in PR/issue responses.
func TestFilterJSONNestedComments(t *testing.T) {
	input := []byte(`{
		"id": "pr-1",
		"title": "Test PR",
		"comments": [
			{"id": "1", "author": {"login": "trusted"}, "body": "good comment"},
			{"id": "2", "author": {"login": "untrusted"}, "body": "bad comment"}
		]
	}`)

	trusted := []string{"trusted"}
	filtered, ignored, err := ghFilterJSON(input, trusted, false)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should have the PR but filtered comments
	if !bytes.Contains(filtered, []byte("Test PR")) {
		t.Error("PR title should be preserved")
	}

	if !bytes.Contains(filtered, []byte("good comment")) {
		t.Error("trusted comment should be in output")
	}

	if bytes.Contains(filtered, []byte("bad comment")) {
		t.Error("untrusted comment should be filtered")
	}

	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	}
}

// TestFilterJSONEmptyTrustedList tests all entries removed when trusted list is empty.
func TestFilterJSONEmptyTrustedList(t *testing.T) {
	input := []byte(`[
		{"id": "1", "author": {"login": "any-user"}, "body": "some comment"}
	]`)

	trusted := []string{}
	filtered, ignored, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// All entries should be filtered
	if len(ignored) != 1 {
		t.Errorf("expected 1 ignored entry, got %d", len(ignored))
	}

	// Output should be empty array
	if !bytes.Equal(bytes.TrimSpace(filtered), []byte("[]")) {
		t.Errorf("expected empty array, got: %s", string(filtered))
	}
}

// TestFilterJSONUserFieldFallback tests user.login fallback when author.login is absent.
func TestFilterJSONUserFieldFallback(t *testing.T) {
	input := []byte(`[
		{"id": "1", "user": {"login": "trusted"}, "body": "comment from user field"}
	]`)

	trusted := []string{"trusted"}
	filtered, _, err := ghFilterJSON(input, trusted, true)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should keep the comment using user.login fallback
	if !bytes.Contains(filtered, []byte("comment from user")) {
		t.Error("comment with user.login should be kept")
	}
}

// TestFilterJSONInvalidJSON tests invalid JSON returns ValueError.
func TestFilterJSONInvalidJSON(t *testing.T) {
	input := []byte(`invalid json {`)

	trusted := []string{"trusted"}
	_, _, err := ghFilterJSON(input, trusted, true)

	if err == nil {
		t.Error("expected error for invalid JSON")
	}

	// err is already a *ValueError from the function signature
}
