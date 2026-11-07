package main

// IdentityFromRun derives an Identity from loaded run metadata and configuration.
// It maps MetaFile variables (AGENT, TYPE, MODEL, TEAM, INV_ID) and Config settings
// (GIT_NAME_FORMAT, GIT_EMAIL, GIT_SIGNING_KEY) to produce a complete Identity ready for git operations.
// runnerID is the task ID of the current run.
// No validation rejects legacy values; all fields are preserved as-is.
func IdentityFromRun(meta MetaFile, cfg Config, runnerID string) *Identity {
	agent, _ := meta.Lookup("AGENT")
	provider, _ := meta.Lookup("TYPE")
	model, _ := meta.Lookup("MODEL")
	team, _ := meta.Lookup("TEAM")
	invID, _ := meta.Lookup("INV_ID")

	emailConfig := cfg.Scalars["GIT_EMAIL"]
	signingKey := cfg.Scalars["GIT_SIGNING_KEY"]

	return &Identity{
		Agent:       agent,
		Provider:    provider,
		Model:       model,
		EmailConfig: emailConfig,
		SigningKey:  signingKey,
		Task:        runnerID,
		Team:        team,
		InvID:       invID,
	}
}
