package main

import (
	"context"
)

// GitHubRunDerivationParams supplies the inputs needed to derive GitHub runner parameters.
type GitHubRunDerivationParams struct {
	// AgentsHome is the root AGENTS_HOME path.
	AgentsHome string
	// TasksDir is the directory containing task metadata directories.
	TasksDir string
	// BusDir is the shared agentbus state directory.
	BusDir string
	// AgentbusBin is the path to the agentbus executable.
	AgentbusBin string
	// RepoRoot is the path to the git repository root.
	RepoRoot string
	// RunnerID is the current task runner identifier.
	RunnerID string
	// Config is the loaded agent configuration.
	Config Config
	// TeamLeadLookup queries the coordinator/lead for a team.
	TeamLeadLookup func(ctx context.Context, team string) (string, error)
	// LookPath resolves executable paths on PATH.
	LookPath func(string) (string, error)
	// SelfExecutable is the path to this executable.
	SelfExecutable string
	// CurrentPath is the inherited PATH environment variable.
	CurrentPath string
	// Logger is the configured application logger.
	Logger *Logger
}

// ResolveCoordinator derives the team coordinator for the runner task.
// If the task is not in a team, or is an investigation, or the lead cannot be determined,
// or the lead is the task's own agent (coordinator itself), it returns an empty string.
func ResolveCoordinator(ctx context.Context, meta MetaFile, leadLookup func(ctx context.Context, team string) (string, error)) string {
	_ = ctx
	_ = meta
	_ = leadLookup
	return ""
}

// ResolveOwnPR determines the PR owned by a solo task or investigation phase 2.
// It returns the PR number as a string, or an empty string if no PR is owned.
func ResolveOwnPR(meta MetaFile) string {
	_ = meta
	return ""
}

// FormatReviewByline formats the investigator review byline.
func FormatReviewByline(agent, typ, model, invID string) string {
	_ = agent
	_ = typ
	_ = model
	_ = invID
	return ""
}

// ResolveReviewByline derives the review byline if the task is an investigation.
// Gated strictly on KIND == "investigation".
func ResolveReviewByline(meta MetaFile) string {
	_ = meta
	return ""
}

// ResolveTeamAndSelf returns the team and self strings for the runner task.
// Cleared to empty if runnerID is empty or KIND starts with "investigation" or TEAM is empty.
func ResolveTeamAndSelf(runnerID string, meta MetaFile) (string, string) {
	_ = runnerID
	_ = meta
	return "", ""
}

// CollectGitHubNames gathers names that can publish on GitHub for color registration.
func CollectGitHubNames(agent string, team string, taskMetas []MetaFile, config Config) []string {
	_ = agent
	_ = team
	_ = taskMetas
	_ = config
	return nil
}

// DeriveGitHubParams derives the GitHubSetupParams from task metadata and configuration.
func DeriveGitHubParams(ctx context.Context, params GitHubRunDerivationParams) (GitHubSetupParams, error) {
	_ = ctx
	_ = params
	return GitHubSetupParams{}, nil
}
