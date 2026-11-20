//! Which refusals of a merge are a conflict with the base of the pull request, and which are any other refusal. A busy
//! or failing GitHub refuses nothing: its errors are not a kind of refusal.

use super::{GitHubError, Rejection};

/// Why GitHub refused to merge a pull request, told apart only as far as the harness acts differently on each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeRefusalKind {
    /// The pull request conflicts with its base, so it cannot be merged as it is.
    Conflict,
    /// Any other refusal that waiting does not change: a required check failed, the head moved, the token may not merge.
    Other,
}

/// Words of GitHub's 405 answers that say the pull request conflicts with its base.
const CONFLICT_MESSAGES: [&str; 3] = ["merge conflicts", "not mergeable", "Base branch was modified"];

/// The kind of refusal `error` is, or `None` when the error is not a refusal of the pull request at all (a rate limit,
/// a failing GitHub, a network failure): those are retried, not told.
#[must_use]
pub fn merge_refusal_kind(error: &GitHubError) -> Option<MergeRefusalKind> {
    let GitHubError::Status { reason, message, .. } = error else {
        return None;
    };
    match *reason {
        Rejection::NotAllowed if CONFLICT_MESSAGES.iter().any(|text| message.contains(text)) => {
            Some(MergeRefusalKind::Conflict)
        }
        Rejection::NotAllowed
        | Rejection::Conflict
        | Rejection::Invalid
        | Rejection::Forbidden
        | Rejection::Unauthorized
        | Rejection::NotFound => Some(MergeRefusalKind::Other),
        Rejection::RateLimited | Rejection::Server | Rejection::Other => None,
    }
}

#[cfg(test)]
mod tests;
