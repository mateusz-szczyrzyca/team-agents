package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
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
	TeamLeadLookup func(ctx context.Context, team string) (string, *ProcessError)
	// LookPath resolves executable paths on PATH.
	LookPath func(string) (string, error)
	// SelfExecutable is the path to this executable.
	SelfExecutable string
	// CurrentPath is the inherited PATH environment variable.
	CurrentPath string
	// Logger is the configured application logger.
	Logger *Logger
}

// metaValue retrieves a key from MetaFile returning empty string if missing.
func metaValue(m MetaFile, key string) string {
	v, _ := m.Lookup(key)
	return v
}

// ResolveCoordinator derives the team coordinator for the runner task.
// If the task is not in a team, or is an investigation, or the lead cannot be determined,
// or the lead is the task's own agent (coordinator itself), it returns an empty string.
func ResolveCoordinator(ctx context.Context, meta MetaFile, leadLookup func(ctx context.Context, team string) (string, *ProcessError)) string {
	team := metaValue(meta, "TEAM")
	if team == "" || strings.HasPrefix(metaValue(meta, "KIND"), "investigation") || leadLookup == nil {
		return ""
	}
	lead, err := leadLookup(ctx, team)
	if err != nil || lead == "" || lead == metaValue(meta, "AGENT") {
		return ""
	}
	return lead
}

// ResolveOwnPR determines the PR owned by a solo task or investigation phase 2.
// It returns the PR number as a string, or an empty string if no PR is owned.
func ResolveOwnPR(meta MetaFile) string {
	if metaValue(meta, "TEAM") != "" {
		return ""
	}
	switch kind := metaValue(meta, "KIND"); {
	case kind == "worker" || kind == "":
	case kind == "investigation" && metaValue(meta, "INVESTIGATION_PHASE") == "2":
	default:
		return ""
	}
	if num := metaValue(meta, "PR_NUMBER"); num != "" {
		return num
	}
	if _, rest, ok := strings.Cut(metaValue(meta, "PR_URL"), "/pull/"); ok {
		if end := strings.IndexFunc(rest, func(r rune) bool { return r < '0' || r > '9' }); end != -1 {
			return rest[:end]
		}
		return rest
	}
	return ""
}

// FormatReviewByline formats the investigator review byline.
func FormatReviewByline(agent, typ, model, invID string) string {
	if model != "" {
		model = ", model " + model
	}
	return fmt.Sprintf("Review by the team-agents investigator `%s` (%s%s), request #%s.", agent, typ, model, invID)
}

// ResolveReviewByline derives the review byline if the task is an investigation.
// Gated strictly on KIND == "investigation".
func ResolveReviewByline(meta MetaFile) string {
	if metaValue(meta, "KIND") == "investigation" {
		return FormatReviewByline(metaValue(meta, "AGENT"), metaValue(meta, "TYPE"), metaValue(meta, "MODEL"), metaValue(meta, "INV_ID"))
	}
	return ""
}

// ResolveTeamAndSelf returns the team and self strings for the runner task.
// Cleared to empty if runnerID is empty or KIND starts with "investigation" or TEAM is empty.
func ResolveTeamAndSelf(runnerID string, meta MetaFile) (string, string) {
	if runnerID == "" || strings.HasPrefix(metaValue(meta, "KIND"), "investigation") || metaValue(meta, "TEAM") == "" {
		return "", ""
	}
	return metaValue(meta, "TEAM"), metaValue(meta, "AGENT")
}

// CollectGitHubNames gathers names that can publish on GitHub for color registration.
func CollectGitHubNames(agent, team string, taskMetas []MetaFile, config Config) []string {
	names := sortedMapKeys(config.Associative["INVESTIGATORS_TYPE"])
	if agent != "" {
		names = append(names, agent)
	}
	for _, m := range taskMetas {
		if team != "" && metaValue(m, "TEAM") == team && metaValue(m, "AGENT") != "" {
			names = append(names, metaValue(m, "AGENT"))
		}
	}
	return names
}

// DeriveGitHubParams derives the GitHubSetupParams from task metadata and configuration.
func DeriveGitHubParams(ctx context.Context, params GitHubRunDerivationParams) (GitHubSetupParams, *StorageError) {
	metaPath := filepath.Join(params.TasksDir, params.RunnerID, "meta.env")
	data, err := os.ReadFile(filepath.Clean(metaPath)) //nolint:gosec // task metadata path within TASKS
	if err != nil {
		return GitHubSetupParams{}, storageErr(err, "read task metadata %s", metaPath)
	}
	meta := parseMeta(data)
	team, self := ResolveTeamAndSelf(params.RunnerID, meta)
	rawColors, _ := configScalar(params.Config, "AGENT_GITHUB_COLORS")
	palette := strings.Fields(rawColors)
	if rawColors == "none" || isPaletteDisabled(palette) {
		palette = nil
	}
	colorsPath := filepath.Join(params.AgentsHome, "github-colors")

	matches, globErr := filepath.Glob(filepath.Join(params.TasksDir, "*", "meta.env"))
	if globErr != nil {
		return GitHubSetupParams{}, storageErr(globErr, "glob task directories in %s", params.TasksDir)
	}
	var logFailures *LogWriteError
	var taskMetas []MetaFile
	for _, p := range matches {
		m, rerr := os.ReadFile(filepath.Clean(p)) //nolint:gosec // task metadata paths
		if rerr != nil {
			if params.Logger != nil {
				logFailures = joinLogFailures(logFailures, params.Logger.logFailure(storageErr(rerr, "read task meta %s", p), "task metadata read"))
			}
			continue
		}
		taskMetas = append(taskMetas, parseMeta(m))
	}
	names := CollectGitHubNames(metaValue(meta, "AGENT"), team, taskMetas, params.Config)

	// Step 1: Register colors under lock first.
	if len(palette) > 0 {
		if colorErr := RegisterGitHubColors(ctx, colorsPath, palette, names, params.Logger); colorErr != nil && params.Logger != nil {
			logFailures = joinLogFailures(logFailures, params.Logger.logFailure(colorErr, "register github colors"))
		}
	}

	// Step 2: Format name colors AFTER registration so newly remembered colors are included.
	formattedNameColors := ""
	if len(palette) > 0 {
		var nameLogErr *LogWriteError
		formattedNameColors, nameLogErr = formatGitHubNameColorsWithLogger(colorsPath, palette, params.Config, params.Logger)
		logFailures = joinLogFailures(logFailures, nameLogErr)
	}

	trusted, _ := configScalar(params.Config, "AGENT_GITHUB_TRUSTED")
	out := GitHubSetupParams{
		AgentsHome: params.AgentsHome, SelfExecutable: params.SelfExecutable, RepoRoot: params.RepoRoot, RunnerID: params.RunnerID,
		Team: team, Agent: self, Coordinator: ResolveCoordinator(ctx, meta, params.TeamLeadLookup), OwnPR: ResolveOwnPR(meta),
		BusDir: params.BusDir, AgentbusBin: params.AgentbusBin, ReviewByline: ResolveReviewByline(meta),
		GitHubTrusted: trusted, GitHubColors: rawColors, GitHubNameColors: formattedNameColors,
		CurrentPath: params.CurrentPath, LookPath: params.LookPath, Logger: params.Logger,
	}
	if logFailures != nil {
		// Nonfatal: logger sink failures are logged to stderr; parameters remain valid.
		return out, nil
	}
	return out, nil
}
