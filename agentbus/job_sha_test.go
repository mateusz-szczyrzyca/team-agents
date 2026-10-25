package main

import (
	"strings"
	"testing"
)

// TestValidJobSHA requires a complete hexadecimal object ID independently of any ref.
func TestValidJobSHA(t *testing.T) {
	cases := []struct {
		name string // identifies the accepted format or rejected input
		sha  string // untrusted object ID supplied by a job or Git
		want bool   // whether the object ID is complete and hexadecimal
	}{
		{"sha1", strings.Repeat("ab", 20), true},
		{"sha256", strings.Repeat("cd", 32), true},
		{"uppercase", strings.Repeat("EF", 20), true},
		{"empty", "", false},
		{"abbreviated", "abcdef01", false},
		{"odd length", strings.Repeat("a", 41), false},
		{"invalid hex", strings.Repeat("g", 40), false},
		{"whitespace", strings.Repeat("a", 40) + "\n", false},
		{"option", "--upload-pack=bad", false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := validJobSHA(tc.sha); got != tc.want {
				t.Fatalf("validJobSHA(%q) = %t, want %t", tc.sha, got, tc.want)
			}
		})
	}
}
