package main

import "testing"

// TestGitHubBranchLinkEscapesRefCharacters keeps valid ref punctuation out of URL controls.
func TestGitHubBranchLinkEscapesRefCharacters(t *testing.T) {
	for _, tc := range []struct {
		branch string // reported branch, including characters Git permits in a ref
		want   string // browser URL with path punctuation encoded and slashes retained
	}{
		{"topic#hash", "https://github.com/acme/repo/tree/topic%23hash"},
		{"topic%value", "https://github.com/acme/repo/tree/topic%25value"},
		{"agents/worker/topic", "https://github.com/acme/repo/tree/agents/worker/topic"},
	} {
		if got := gitHubBranchURL("https://github.com/acme/repo", tc.branch); got != tc.want {
			t.Errorf("branch %q: got %q, want %q", tc.branch, got, tc.want)
		}
	}
}
