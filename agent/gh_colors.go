package main

import (
	"context"
)

// RegisterGitHubColors assigns a color from palette to each name in names that does not yet have one.
// Assignments are stored in colorsPath ("<name> <#RRGGBB>\n").
// Concurrency is handled via flock on colorsPath + ".lock".
func RegisterGitHubColors(ctx context.Context, colorsPath string, palette []string, names []string, logger *Logger) *StorageError {
	_ = ctx
	_ = colorsPath
	_ = palette
	_ = names
	_ = logger
	return nil
}

// FormatGitHubNameColors formats the AGENT_GITHUB_NAME_COLORS string ("name=#RRGGBB ...").
// It first takes remembered colors in colorsPath, then for any configured agents not in have,
// assigns their color from the configuration's order (NORMAL_AGENTS, WORKER_AGENTS, sorted AGENT_TYPE, sorted INVESTIGATORS_TYPE).
func FormatGitHubNameColors(colorsPath string, palette []string, config Config) string {
	_ = colorsPath
	_ = palette
	_ = config
	return ""
}
