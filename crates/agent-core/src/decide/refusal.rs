//! Refusal details from a provider's structured response.
//!
//! When the CLI gives structured refusal data (opencode's error event, Claude's `rate_limit_event`, Codex's
//! `turn.failed`), this module holds the structured summary; when not, the text-based [`Pause::evidence`] is
//! the only record.
//!
//! [`Pause::evidence`]: super::outcome::Pause::evidence

use super::outcome::PauseReason;

/// What the provider said when it refused a run, taken from structured fields of the CLI's output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalSummary {
    /// The HTTP status code as a string, e.g. `"429"`; empty when not told.
    pub status: String,
    /// The provider's message, e.g. `"Model busy, retry later"`; empty when not told.
    pub message: String,
    /// The provider's code, e.g. `"engine_overloaded"`; empty when not told.
    pub code: String,
    /// Which limit the refusal is: an overload for 429/503/529, else a rate limit or a quota.
    pub kind: RefusalKind,
}

/// The kind of refusal a provider gave.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, strum::Display, strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RefusalKind {
    /// An exhausted quota, usage limit or credits.
    Quota,
    /// The provider is rate-limiting requests.
    RateLimit,
    /// The model or provider is overloaded.
    Overloaded,
    /// A provider error that is neither a quota nor a rate limit nor an overload.
    ProviderError,
}

impl RefusalSummary {
    /// Builds a refusal summary from the fields the CLI gave.
    ///
    /// `status` is the HTTP status code as a string (`"429"`, `"402"`, or empty),
    /// `message` is the provider's message, `code` is the provider's error code.
    /// The `kind` is inferred from the status and the pause reason.
    #[must_use]
    pub fn from_fields(status: &str, message: &str, code: &str, reason: PauseReason) -> Self {
        let kind = refusal_kind(status, reason);
        Self {
            status: status.to_owned(),
            message: message.to_owned(),
            code: code.to_owned(),
            kind,
        }
    }
}

/// Infers the [`RefusalKind`] from the HTTP status and the pause reason.
#[must_use]
fn refusal_kind(status: &str, reason: PauseReason) -> RefusalKind {
    if reason == PauseReason::Quota {
        return RefusalKind::Quota;
    }
    // Overloaded: 503, 529
    if matches!(status, "503" | "529") {
        return RefusalKind::Overloaded;
    }
    // Rate limit: 429, or rate-limited reason with no specific overload/error status
    if matches!(status, "429" | "") {
        RefusalKind::RateLimit
    } else {
        RefusalKind::ProviderError
    }
}

#[cfg(test)]
mod tests;
