package main

import (
	"strings"
	"testing"
)

// TestIdentityEmptyModel tests that empty model produces no slash in Agent-Model trailer.
func TestIdentityEmptyModel(t *testing.T) {
	id := &Identity{Agent: "test", Provider: "p1", Model: "", Task: "t1"}
	trailers := id.Trailers()

	for _, trailer := range trailers {
		if strings.HasPrefix(trailer, "Agent-Model:") {
			// Should be "Agent-Model: p1" not "Agent-Model: p1/"
			if strings.HasSuffix(trailer, "/") {
				t.Errorf("Empty model should not produce slash, got: %s", trailer)
			}
			if !strings.Contains(trailer, "p1") {
				t.Errorf("Expected provider in trailer, got: %s", trailer)
			}
		}
	}
}

// TestIdentityUnreadableKeyEnv tests unreadable key produces disabled signing config.
func TestIdentityUnreadableKeyEnv(t *testing.T) {
	id := &Identity{Agent: "a1", SigningKey: "/nonexistent/key/path"}
	env := id.SigningEnv()

	if env == nil {
		t.Fatal("SigningEnv returned nil")
	}
	if env["GIT_CONFIG_COUNT"] != "2" {
		t.Errorf("Unreadable key: GIT_CONFIG_COUNT = %q, want 2", env["GIT_CONFIG_COUNT"])
	}
	if env["GIT_CONFIG_VALUE_0"] != "false" {
		t.Errorf("Unreadable key: signing not disabled, got VALUE_0=%q", env["GIT_CONFIG_VALUE_0"])
	}
}

// TestIdentityNoGlobalMutation verifies returned environment is independent of process state.
func TestIdentityNoGlobalMutation(t *testing.T) {
	id := &Identity{Agent: "a1", Provider: "p1", Model: "m1", Task: "t1"}

	// Get environment
	env1 := id.Environment()
	original := make(map[string]string)
	for k, v := range env1 {
		original[k] = v
	}

	// Mutate returned map
	env1["GIT_AUTHOR_NAME"] = "MODIFIED"
	env1["GIT_AUTHOR_EMAIL"] = "modified@test"

	// Get environment again - should not be affected by mutation
	env2 := id.Environment()
	if env2["GIT_AUTHOR_NAME"] == "MODIFIED" {
		t.Error("Environment() returned map should not be affected by external mutation")
	}
	if env2["GIT_AUTHOR_NAME"] != original["GIT_AUTHOR_NAME"] {
		t.Errorf("Environment() not deterministic after mutation: %q != %q",
			env2["GIT_AUTHOR_NAME"], original["GIT_AUTHOR_NAME"])
	}
}
