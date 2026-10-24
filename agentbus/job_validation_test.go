package main

import (
	"strings"
	"testing"
)

// TestJobIdentifierBoundary keeps network job names bounded and separate from paths.
func TestJobIdentifierBoundary(t *testing.T) {
	for _, name := range []string{"a", "node-2", "repo.v1_2", strings.Repeat("a", 128)} {
		if !validJobIdentifier(name) {
			t.Errorf("valid identifier rejected: %q", name)
		}
	}
	for _, name := range []string{"", "-option", ".", "..", "a/b", "a b", "a\n", "a@b", strings.Repeat("a", 129)} {
		if validJobIdentifier(name) {
			t.Errorf("invalid identifier accepted: %q", name)
		}
	}
}

// TestJobBaseValidation rejects Git revision expressions and option-shaped refs.
func TestJobBaseValidation(t *testing.T) {
	for _, ref := range []string{"main", "refs/heads/main", "agents/node-2/task", "release/v1.2"} {
		if !validJobBase(ref, "") {
			t.Errorf("valid ref rejected: %q", ref)
		}
	}
	for _, ref := range []string{"", "-main", "--upload-pack=x", "main~1", "main^", "main:foo", "main?", "main*", "a[b", "a\\b", "a b", "a\n", "a\x7f", "a..b", "a@{b", "@", "/main", "main/", "main.", "a//b", ".main", "a/.b", "a.lock", "a.lock/b"} {
		if validJobBase(ref, "") {
			t.Errorf("invalid ref accepted: %q", ref)
		}
	}
}

// TestJobBaseExactSHA permits an optional SHA-1 or SHA-256 and no revision syntax.
func TestJobBaseExactSHA(t *testing.T) {
	for _, sha := range []string{"", strings.Repeat("a", 40), strings.Repeat("F", 64)} {
		if !validJobBase("main", sha) {
			t.Errorf("valid SHA rejected: %q", sha)
		}
	}
	for _, sha := range []string{"HEAD", strings.Repeat("a", 39), strings.Repeat("a", 41), strings.Repeat("g", 40), "-" + strings.Repeat("a", 39)} {
		if validJobBase("main", sha) {
			t.Errorf("invalid SHA accepted: %q", sha)
		}
	}
}
