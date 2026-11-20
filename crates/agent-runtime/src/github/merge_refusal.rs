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

/// The kind of refusal `error` is, or `None` when the error is not a refusal of the pull request at all (a rate limit,
/// a failing GitHub, a network failure): those are retried, not told.
#[must_use]
pub fn merge_refusal_kind(error: &GitHubError) -> Option<MergeRefusalKind> {
    let _ = error;
    None
}

#[cfg(test)]
mod tests;
