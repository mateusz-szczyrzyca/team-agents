package main

import (
	"fmt"
	"os"
	"strings"
)

// Identity holds agent identity information for git commits and environment setup.
// It manages author/committer names, email formatting, signing configuration, and commit trailers.
type Identity struct {
	// Agent is the agent's name (e.g., "claude-haiku", "codex-1").
	Agent string
	// Provider is the provider type (e.g., "claude", "codex", "gemini").
	Provider string
	// Model is the model identifier (e.g., "haiku", "gpt-6.1-sol").
	Model string
	// EmailConfig is the configured email template or fixed address.
	EmailConfig string
	// SigningKey is the path to the SSH signing key, if any.
	SigningKey string
	// Task is the task ID this run belongs to.
	Task string
	// Team is the team ID, if this is a team run.
	Team string
	// InvID is the investigation ID, if this is an investigator run.
	InvID string
}

// FormatAuthor formats the author name according to the provided format string,
// expanding %a to the agent name.
func (id *Identity) FormatAuthor(format string) string {
	return strings.ReplaceAll(format, "%a", id.Agent)
}

// Email returns the email address for this identity, using the configured
// template with %a substitution, or the default .invalid address if empty.
func (id *Identity) Email() string {
	if id.EmailConfig == "" {
		return id.Agent + "@team-agents.invalid"
	}
	return strings.ReplaceAll(id.EmailConfig, "%a", id.Agent)
}

// SigningEnv returns environment variables for git signing configuration.
// Uses oracle GIT_CONFIG_COUNT pattern with indexed KEY/VALUE pairs.
func (id *Identity) SigningEnv() map[string]string {
	env := make(map[string]string)

	if id.SigningKey != "" {
		if _, err := os.Stat(id.SigningKey); err == nil {
			// Key exists: enable signing with full config
			env["GIT_CONFIG_COUNT"] = "4"
			env["GIT_CONFIG_KEY_0"] = "gpg.format"
			env["GIT_CONFIG_VALUE_0"] = "ssh"
			env["GIT_CONFIG_KEY_1"] = "user.signingkey"
			env["GIT_CONFIG_VALUE_1"] = id.SigningKey
			env["GIT_CONFIG_KEY_2"] = "commit.gpgsign"
			env["GIT_CONFIG_VALUE_2"] = "true"
			env["GIT_CONFIG_KEY_3"] = "tag.gpgsign"
			env["GIT_CONFIG_VALUE_3"] = "true"
			return env
		}
	}

	// Key missing or unreadable: disable signing (GIT_CONFIG_COUNT=2 for gpgsign false)
	env["GIT_CONFIG_COUNT"] = "2"
	env["GIT_CONFIG_KEY_0"] = "commit.gpgsign"
	env["GIT_CONFIG_VALUE_0"] = "false"
	env["GIT_CONFIG_KEY_1"] = "tag.gpgsign"
	env["GIT_CONFIG_VALUE_1"] = "false"
	return env
}

// Trailers returns the commit trailers as a list of lines.
// Required trailers: Agent, Agent-Model, Task.
// Optional trailers: Team, Investigation (if set).
func (id *Identity) Trailers() []string {
	var trailers []string

	trailers = append(trailers, fmt.Sprintf("Agent: %s", id.Agent))
	trailers = append(trailers, fmt.Sprintf("Agent-Model: %s/%s", id.Provider, id.Model))
	trailers = append(trailers, fmt.Sprintf("Task: %s", id.Task))

	if id.Team != "" {
		trailers = append(trailers, fmt.Sprintf("Team: %s", id.Team))
	}

	if id.InvID != "" {
		trailers = append(trailers, fmt.Sprintf("Investigation: #%s", id.InvID))
	}

	return trailers
}

// Environment returns all environment variables needed for git operations
// with this identity (author, committer, email, signing config).
func (id *Identity) Environment() map[string]string {
	env := make(map[string]string)

	author := id.FormatAuthor("%a (team-agents)")
	email := id.Email()

	env["GIT_AUTHOR_NAME"] = author
	env["GIT_AUTHOR_EMAIL"] = email
	env["GIT_COMMITTER_NAME"] = author
	env["GIT_COMMITTER_EMAIL"] = email

	// Merge signing configuration
	for k, v := range id.SigningEnv() {
		env[k] = v
	}

	return env
}
