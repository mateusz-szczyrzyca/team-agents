package main

import (
	"context"
)

// CanonicalGitHubRepoURL parses a git remote origin URL and returns the
// canonical GitHub HTTPS form (https://github.com/owner/repo), or empty
// for non-GitHub origins or invalid formats.
func CanonicalGitHubRepoURL(_ string) string {
	return ""
}

// ResolveGitHubRepoURL resolves the repository's GitHub origin by reading
// git remote get-url origin and parsing it to canonical form.
// Returns empty for missing origin (silent), non-GitHub origins, or errors.
func ResolveGitHubRepoURL(_ context.Context, _ Executor, _ string, _ *Logger) string {
	return ""
}
