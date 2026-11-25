//! What a provider's refusal says, read from the fields of the CLI's own record when it has them.
//!
//! A run the provider refused leaves a record in its output: opencode's `error` record with the provider's status and
//! the body of its response, Claude Code's `rate_limit_event` and error `result`, codex's `error` message. [`describe`]
//! turns one such record into the words of a notice: the kind of refusal, then the status, the message and the provider's
//! code. The kind comes from the provider's fields, never from a constant. A line that is no such record is not
//! described: the caller shows it as it is.

#[cfg(test)]
mod tests;

/// The kind of a provider's refusal, as the provider's fields say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The model is busy or overloaded: a pause as short as a rate limit's.
    Overload,
    /// The provider limits the rate of requests.
    RateLimit,
    /// The quota, usage limit or credits are used up.
    Quota,
}

impl Kind {
    /// The words of this kind in a notice, with its article: `an overload`, `a rate limit`, `a quota`.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::Overload => "an overload",
            Self::RateLimit => "a rate limit",
            Self::Quota => "a quota",
        }
    }
}

/// The notice text of the refusal that `line` records: `<kind>: <status> · <message> · <code>`, with the parts the record
/// has, or `None` when `line` is not a refusal record of a known shape (the caller then shows the line itself).
#[must_use]
pub fn describe(_line: &str) -> Option<String> {
    None
}
