//! Reviews: who reviews a pull request, what a reviewer is asked and what its report says.
//!
//! The reviewers are investigators from outside the team. A review is a task of the team (`Team::reviewers`) that ends
//! `waiting`, so that the same investigator can review the next pull request and the same one again after a fix pass.
//! Everything here decides from the state and the text of a report; the owner carries it out (`owner/rounds.rs`).

use agent_core::status::ReviewVerdict;

/// The longest report the harness keeps, posts and hands on, in characters: far above any real report, and under the
/// limit of a comment on GitHub.
pub(super) const MAX_REPORT_CHARS: usize = 20_000;

/// Why a reviewer's final message gives no verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VerdictProblem {
    /// No line of the message is a `VERDICT:` line.
    Missing,
    /// The first `VERDICT:` line has something other than one of the two verdicts.
    Unreadable {
        /// What the line says after `VERDICT:`, cut.
        found: String,
    },
}

/// The verdict of a reviewer's final message: the first `VERDICT:` line, once the markup characters are dropped, says
/// `APPROVE` or `REQUEST_CHANGES` and nothing else.
///
/// # Errors
/// [`VerdictProblem`].
pub(super) fn parse_verdict(message: &str) -> Result<ReviewVerdict, VerdictProblem> {
    let _ = message;
    Err(VerdictProblem::Missing)
}

/// The report in a reviewer's final message, as it is posted and kept: without the `STATUS:` line, at most
/// [`MAX_REPORT_CHARS`] characters.
pub(super) fn report_text(message: &str) -> String {
    let _ = message;
    String::new()
}

#[cfg(test)]
mod tests;
