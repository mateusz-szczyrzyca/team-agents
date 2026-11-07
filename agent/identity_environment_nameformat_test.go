package main

import "testing"

// TestIdentityEnvironmentHonorsNameFormat verifies that Environment honors configured NameFormat.
func TestIdentityEnvironmentHonorsNameFormat(t *testing.T) {
	id := &Identity{
		Agent:      "alice",
		NameFormat: "CUSTOM:%a",
	}
	env := id.Environment()
	if env["GIT_AUTHOR_NAME"] != "CUSTOM:alice" || env["GIT_COMMITTER_NAME"] != "CUSTOM:alice" {
		t.Fatalf("unexpected author in environment: %q", env["GIT_AUTHOR_NAME"])
	}
}
