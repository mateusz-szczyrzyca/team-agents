package main

import (
	"strings"
	"testing"
)

func TestIdentityEnvironmentHonorsNameFormat(t *testing.T) {
	// RED: Environment should use configured NameFormat, not hardcoded format
	id := &Identity{
		Agent:       "test-agent",
		Provider:    "test",
		Model:       "test-model",
		NameFormat:  "CUSTOM:%a",
		EmailConfig: "test@example.com",
		Task:        "test-task",
	}

	env := id.Environment()

	// Environment should use CUSTOM:%a format, not "%a (team-agents)"
	author := env["GIT_AUTHOR_NAME"]
	if !strings.Contains(author, "CUSTOM:") {
		t.Errorf("Environment should honor NameFormat; got author %q, want to contain CUSTOM:", author)
	}
	if !strings.Contains(author, "test-agent") {
		t.Errorf("Environment author should contain agent name; got %q", author)
	}
}
