//! The kinds of refused merges, from the status and the message GitHub gives.

use super::super::{GitHubError, GitHubOperation, Rejection};
use super::*;

/// A refusal of the merge call with `status`, `reason` and GitHub's `message`.
fn refused(status: u16, reason: Rejection, message: &str) -> GitHubError {
    GitHubError::Status {
        operation: GitHubOperation::MergePullRequest,
        status,
        reason,
        retry_after: None,
        message: message.to_owned(),
    }
}

#[test]
fn a_405_that_says_the_pull_request_has_merge_conflicts_is_a_conflict() {
    let error = refused(405, Rejection::NotAllowed, "Pull Request has merge conflicts");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Conflict));
}

#[test]
fn a_405_that_says_the_pull_request_is_not_mergeable_is_a_conflict() {
    let error = refused(405, Rejection::NotAllowed, "Pull Request is not mergeable");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Conflict));
}

#[test]
fn a_405_that_says_the_base_branch_was_modified_is_a_conflict() {
    let error = refused(405, Rejection::NotAllowed, "Base branch was modified");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Conflict));
}

#[test]
fn a_405_about_a_moved_head_is_another_refusal() {
    let error = refused(405, Rejection::NotAllowed, "Head branch was modified");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Other));
}

#[test]
fn a_405_about_a_failed_required_check_is_another_refusal() {
    let error = refused(405, Rejection::NotAllowed, "Required status check \"ci\" is expected.");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Other));
}

#[test]
fn a_409_is_another_refusal_even_when_it_names_a_conflict() {
    let error = refused(409, Rejection::Conflict, "Pull Request has merge conflicts");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Other));
}

#[test]
fn a_forbidden_merge_is_another_refusal() {
    let error = refused(403, Rejection::Forbidden, "Resource not accessible by integration");

    assert_eq!(merge_refusal_kind(&error), Some(MergeRefusalKind::Other));
}

#[test]
fn a_rate_limit_and_a_failing_github_are_no_refusal() {
    let limited = refused(429, Rejection::RateLimited, "API rate limit exceeded");
    let failing = refused(502, Rejection::Server, "Bad gateway");

    assert_eq!(merge_refusal_kind(&limited), None);
    assert_eq!(merge_refusal_kind(&failing), None);
}
