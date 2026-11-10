package main

// FormatAgentDescriptor returns the formatted agent descriptor matching Bash agent_desc:
// "`name` (type)" or "`name` (type, model `model`)" depending on whether model is set.
// Type is looked up from AGENT_TYPE or INVESTIGATORS_TYPE, defaulting to "?" if missing.
// Model is looked up from AGENT_MODEL or INVESTIGATORS_MODEL independently.
func FormatAgentDescriptor(_ string, _ *Config) string {
	return ""
}
