//! Whether a failed call of the GitHub API may pass when it is made again later: the network, GitHub itself and a rate
//! limit may; a refusal GitHub will give again (a missing pull request, a token that may not do it) may not.

use super::{GitHubError, Rejection};

#[cfg(test)]
mod tests;

/// The outlook of a call of the GitHub API that failed: whether a later try may do better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outlook {
    /// A later try may pass: nothing answered, the answer broke, the call ran over its time, GitHub failed or
    /// limited the account.
    MayPass,
    /// A later try gives the same answer until someone changes something: the request, the token or the repository.
    WillNotPass,
}

impl GitHubError {
    /// Whether a call that failed with this error is worth making again later.
    #[must_use]
    pub fn outlook(&self) -> Outlook {
        match self {
            Self::Transport { .. } | Self::Timeout { .. } => Outlook::MayPass,
            Self::Status {
                reason: Rejection::Server | Rejection::RateLimited,
                ..
            } => Outlook::MayPass,
            Self::Status { reason, .. } => match reason {
                Rejection::Server | Rejection::RateLimited => Outlook::MayPass,
                Rejection::Unauthorized
                | Rejection::Forbidden
                | Rejection::NotFound
                | Rejection::NotAllowed
                | Rejection::Conflict
                | Rejection::Invalid
                | Rejection::Other => Outlook::WillNotPass,
            },
            Self::Client { .. }
            | Self::ResponseTooLarge { .. }
            | Self::Decode { .. }
            | Self::GraphQl { .. }
            | Self::StillDraft { .. }
            | Self::TooManyPages { .. }
            | Self::NothingToUpdate => Outlook::WillNotPass,
        }
    }
}
