package main

import (
	"context"
	"regexp"
	"strconv"
	"strings"
)

// ownerPattern validates owner: alphanumeric/dash, starts with alphanumeric, max 39 chars.
var ownerPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9-]{0,38}$`)

// repoPattern validates repo: alphanumeric/dash/dot/underscore, not . or .., max 100 chars.
var repoPattern = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,100}$`)

// CanonicalGitHubRepoURL parses a git remote origin URL and returns the
// canonical GitHub HTTPS form (https://github.com/owner/repo), or empty
// for non-GitHub origins or invalid formats.
func CanonicalGitHubRepoURL(remoteURL string) string {
	var tail string
	switch {
	case strings.HasPrefix(remoteURL, "https://github.com/"):
		tail = strings.TrimPrefix(remoteURL, "https://github.com/")
	case strings.HasPrefix(remoteURL, "git@github.com:"):
		tail = strings.TrimPrefix(remoteURL, "git@github.com:")
	case strings.HasPrefix(remoteURL, "ssh://git@github.com/"):
		tail = strings.TrimPrefix(remoteURL, "ssh://git@github.com/")
	default:
		return ""
	}

	tail = strings.TrimSuffix(tail, ".git")
	slash := strings.IndexByte(tail, '/')
	if slash == -1 {
		return ""
	}
	owner := tail[:slash]
	repo := tail[slash+1:]

	if !ownerPattern.MatchString(owner) || !repoPattern.MatchString(repo) {
		return ""
	}
	if repo == "." || repo == ".." {
		return ""
	}

	return "https://github.com/" + owner + "/" + repo
}

// ResolveGitHubRepoURL resolves the repository's GitHub origin by reading
// git remote get-url origin and parsing it to canonical form.
// Returns empty for missing origin (silent), non-GitHub origins, or errors.
func ResolveGitHubRepoURL(ctx context.Context, exec Executor, repoRoot string, logger *Logger) string {
	if exec == nil {
		exec = OSExecutor{}
	}

	var out strings.Builder
	r, err := exec.Execute(ctx, Command{
		Name:   "git",
		Args:   []string{"-C", repoRoot, "remote", "get-url", "origin"},
		Stdout: &out,
		Stderr: &out,
	})

	exitCode := 0
	if err != nil {
		exitCode = 1
	} else {
		exitCode = r.ExitCode
	}

	raw := out.String()
	if raw == "" && err != nil {
		raw = err.Error()
	}
	if exitCode == 2 {
		return ""
	}
	if exitCode != 0 {
		if logger != nil {
			reason := raw
			runes := []rune(reason)
			if len(runes) > 300 {
				reason = string(runes[:300])
			}
			if logErr := logger.Log(ctx, LevelWarn, LogJobOriginLookupFailed, "repository="+repoRoot, "exit="+strconv.Itoa(exitCode), "reason="+reason); logErr != nil {
				// Side effect: log write failure is nonfatal per the Bash oracle; Logger.Log already emits diagnostics.
				return ""
			}
		}
		return ""
	}

	cleaned := strings.TrimRight(raw, "\n")
	return CanonicalGitHubRepoURL(cleaned)
}
