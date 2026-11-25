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
    /// The Unix epoch second when the limit resets, from a structured event of the CLI; none when not told or when
    /// the reset is parsed from text.
    pub resets_at: Option<u64>,
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
    /// `message` is the provider's message, `code` is the provider's error code,
    /// `resets_at` is the Unix epoch second of the reset from a structured event, and
    /// `reason` is how the run's limit was classified (`Quota` or `RateLimit`).
    /// The `kind` is inferred from the status, the code and the pause reason.
    #[must_use]
    pub fn from_fields(
        status: &str,
        message: &str,
        code: &str,
        resets_at: Option<u64>,
        reason: PauseReason,
    ) -> Self {
        let kind = refusal_kind(status, code, reason);
        Self {
            status: status.to_owned(),
            message: message.to_owned(),
            code: code.to_owned(),
            kind,
            resets_at,
        }
    }
}

/// The provider's error codes that, in lower case, show a model overloaded.
const OVERLOAD_CODES: [&str; 2] = ["overloaded", "at_capacity"];

/// Infers the [`RefusalKind`] from the HTTP status, the provider's error code and the pause reason.
#[must_use]
fn refusal_kind(status: &str, code: &str, reason: PauseReason) -> RefusalKind {
    if reason == PauseReason::Quota {
        return RefusalKind::Quota;
    }
    let lower_code = code.to_lowercase();
    // Overloaded: 503, 529, or a code that says so (e.g. engine_overloaded).
    if matches!(status, "503" | "529")
        || OVERLOAD_CODES.iter().any(|phrase| lower_code.contains(phrase))
    {
        return RefusalKind::Overloaded;
    }
    // Rate limit: 429, or rate-limited reason with no specific overload/error status.
    if matches!(status, "429" | "") {
        RefusalKind::RateLimit
    } else {
        RefusalKind::ProviderError
    }
}

#[cfg(test)]
mod tests;
