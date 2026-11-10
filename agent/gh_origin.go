package main

import (
	"context"
	"regexp"
	"strings"
)

// originPattern matches HTTPS, git@, and ssh:// GitHub URLs and extracts components.
var originPattern = regexp.MustCompile(`^(?:https://|git@|ssh://git@)github\.com[:/]([^/]+)/(.+?)(?:\.git)?$`)

// ownerPattern validates owner: alphanumeric/dash, starts with alphanumeric, max 39 chars.
var ownerPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9-]{0,38}$`)

// repoPattern validates repo: alphanumeric/dash/dot/underscore, not . or .., max 100 chars.
var repoPattern = regexp.MustCompile(`^[A-Za-z0-9_.-]+$`)

// CanonicalGitHubRepoURL parses a git remote origin URL and returns the
// canonical GitHub HTTPS form (https://github.com/owner/repo), or empty
// for non-GitHub origins or invalid formats.
func CanonicalGitHubRepoURL(remoteURL string) string {
	trimmed := strings.TrimSpace(remoteURL)
	m := originPattern.FindStringSubmatch(trimmed)
	if len(m) < 3 {
		return ""
	}

	owner := m[1]
	repo := m[2]

	// Strip trailing .git if present (already done by regex (?:\.git)?)
	// Validate owner and repo
	if !ownerPattern.MatchString(owner) || !repoPattern.MatchString(repo) {
		return ""
	}

	// Reject . and ..
	if repo == "." || repo == ".." {
		return ""
	}

	return "https://github.com/" + owner + "/" + repo
}

// ResolveGitHubRepoURL resolves the repository's GitHub origin by reading
// git remote get-url origin and parsing it to canonical form.
// Returns empty for missing origin (silent), non-GitHub origins, or errors.
func ResolveGitHubRepoURL(ctx context.Context, exec Executor, repoRoot string, _ *Logger) string {
	if exec == nil {
		exec = OSExecutor{}
	}

	var out strings.Builder
	r, err := exec.Execute(ctx, Command{
		Name:   "git",
		Args:   []string{"-C", repoRoot, "remote", "get-url", "origin"},
		Stdout: &out,
	})

	// git exit 2 means missing origin - silent fail
	if err != nil || (r.ExitCode == 2) {
		return ""
	}

	// Other git failures - silent fail (no log)
	if r.ExitCode != 0 {
		return ""
	}

	remoteURL := strings.TrimSpace(out.String())
	canonical := CanonicalGitHubRepoURL(remoteURL)

	return canonical
}
