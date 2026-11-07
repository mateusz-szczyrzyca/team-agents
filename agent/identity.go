package main

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
func (id *Identity) FormatAuthor(_ string) string {
	return ""
}

// Email returns the email address for this identity, using the configured
// template with %a substitution, or the default .invalid address if empty.
func (id *Identity) Email() string {
	return ""
}

// SigningEnv returns environment variables for git signing configuration.
// If SigningKey is set and exists, signing is enabled; otherwise disabled.
func (id *Identity) SigningEnv() map[string]string {
	return nil
}

// Trailers returns the commit trailers as a list of lines.
// Required trailers: Agent, Agent-Model, Task.
// Optional trailers: Team, Investigation (if set).
func (id *Identity) Trailers() []string {
	return nil
}

// Environment returns all environment variables needed for git operations
// with this identity (author, committer, email, signing config).
func (id *Identity) Environment() map[string]string {
	return nil
}
