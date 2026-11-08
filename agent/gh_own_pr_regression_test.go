package main

import "testing"

// TestResolveOwnPRLastDelimiter preserves Bash parsing for owners and repositories named pull.
func TestResolveOwnPRLastDelimiter(t *testing.T) {
	for _, suffix := range []string{"", "/files"} {
		t.Run(suffix, func(t *testing.T) {
			meta := parseMeta([]byte("KIND=worker\nPR_URL=https://github.com/pull/pull/pull/456" + suffix + "\n"))
			if got := ResolveOwnPR(meta); got != "456" {
				t.Fatalf("ResolveOwnPR() = %q, want 456", got)
			}
		})
	}
}
