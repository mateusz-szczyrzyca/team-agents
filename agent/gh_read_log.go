package main

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"time"
)

// EventGitHubCommentIgnored records a comment from an untrusted author.
const EventGitHubCommentIgnored LogEvent = "github_comment_ignored"

// EventGitHubCommentMarkedIgnored records an explicitly ignored trusted comment.
const EventGitHubCommentMarkedIgnored LogEvent = "github_comment_marked_ignored"

// EventGitHubPRIgnored records a filtered pull request in a list.
const EventGitHubPRIgnored LogEvent = "github_pr_ignored"

// EventGitHubIssueIgnored records a filtered issue in a list.
const EventGitHubIssueIgnored LogEvent = "github_issue_ignored"

// EventPRUntrustedIgnored records a refused read of an untrusted target.
const EventPRUntrustedIgnored LogEvent = "pr_untrusted_ignored"

// EventGitHubGraphQLRefused records a query that bypasses filtering.
const EventGitHubGraphQLRefused LogEvent = "github_graphql_refused"

// EventGitHubMemberPRRefused records a member attempting coordinator work.
const EventGitHubMemberPRRefused LogEvent = "github_member_pr_refused"

// EventGitHubSecondPRRefused records an attempt to create another round PR.
const EventGitHubSecondPRRefused LogEvent = "github_second_pr_refused"

// EventGitHubCoordinatorUnknown records an unresolved current coordinator.
const EventGitHubCoordinatorUnknown LogEvent = "github_coordinator_unknown"

// ghReadLog appends the wrapper's warning schema; callers report side-effect failures.
func ghReadLog(env []string, event LogEvent, message string, fields map[string]string) *StorageError {
	record := map[string]string{"ts": time.Now().UTC().Format("2006-01-02T15:04:05Z"), "level": "warn", "component": "agent-gh", "event": string(event), "message": message}
	for key, value := range fields {
		record[key] = value
	}
	line, err := json.Marshal(record)
	if err != nil {
		return storageErr(err, "encode GitHub warning")
	}
	home := ghEnvValue(env, "AGENTS_HOME")
	if home == "" {
		home = filepath.Join(ghEnvValue(env, "HOME"), "agents")
	}
	return appendLog(filepath.Join(home, "logs", "agent.jsonl"), append(line, '\n'))
}

// ghEnvValue reads the supplied process environment without consulting global state.
func ghEnvValue(env []string, name string) string {
	for i := len(env) - 1; i >= 0; i-- {
		if value, ok := strings.CutPrefix(env[i], name+"="); ok {
			return value
		}
	}
	return ""
}
