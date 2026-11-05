package main

import "io"

// cmdAgents prints configured agents and current allocation using explicit state paths.
func (a *App) cmdAgents(_ io.Writer, _ *Config, _, _ string) *AgentsError {
	return nil
}

// cmdAgentsJSON emits one configuration record per agent and investigator.
func (a *App) cmdAgentsJSON(_ io.Writer, _ *Config) *AgentsJSONError {
	return nil
}
