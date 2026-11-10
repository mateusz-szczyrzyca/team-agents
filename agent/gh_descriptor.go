package main

// FormatAgentDescriptor returns the formatted agent descriptor matching Bash agent_desc:
// "`name` (type)" or "`name` (type, model `model`)" depending on whether model is set.
// Type is looked up from AGENT_TYPE or INVESTIGATORS_TYPE, defaulting to "?" if missing.
// Model is looked up from AGENT_MODEL or INVESTIGATORS_MODEL independently.
func FormatAgentDescriptor(name string, cfg *Config) string {
	// Lookup type: AGENT_TYPE first, then INVESTIGATORS_TYPE, default to "?"
	agentTypes := cfg.Associative["AGENT_TYPE"]
	typ := agentTypes[name]
	if typ == "" {
		investigatorTypes := cfg.Associative["INVESTIGATORS_TYPE"]
		typ = investigatorTypes[name]
		if typ == "" {
			typ = "?"
		}
	}

	// Lookup model: AGENT_MODEL first, then INVESTIGATORS_MODEL
	agentModels := cfg.Associative["AGENT_MODEL"]
	model := agentModels[name]
	if model == "" {
		investigatorModels := cfg.Associative["INVESTIGATORS_MODEL"]
		model = investigatorModels[name]
	}

	// Format output: `name` (type) or `name` (type, model `model`)
	if model == "" {
		return "`" + name + "` (" + typ + ")"
	}
	return "`" + name + "` (" + typ + ", model `" + model + "`)"
}
