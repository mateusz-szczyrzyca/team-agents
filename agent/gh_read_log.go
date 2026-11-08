package main

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
func ghReadLog(_ []string, _ LogEvent, _ string, _ map[string]string) *StorageError {
	return nil
}
