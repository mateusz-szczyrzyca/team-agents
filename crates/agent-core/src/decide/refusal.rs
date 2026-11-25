//! What a provider's refusal says, read from the fields of the CLI's own record when it has them.
//!
//! A run the provider refused leaves a record in its output: opencode's `error` record with the provider's status and
//! the body of its response, Claude Code's `rate_limit_event` and error `result`, codex's `error` message. [`describe`]
//! turns one such record into the words of a notice: the kind of refusal, then the status, the message and the provider's
//! code. The kind comes from the provider's fields, never from a constant. A line that is no such record is not
//! described: the caller shows it as it is.

use serde_json::Value;

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

/// Separator between the parts of a description.
const SEPARATOR: &str = " · ";

/// The HTTP status of a quota that is used up.
const PAYMENT_REQUIRED: u64 = 402;

/// The HTTP status of a rate limit.
const TOO_MANY_REQUESTS: u64 = 429;

/// The HTTP statuses of an overloaded model.
const OVERLOADED_STATUSES: [u64; 2] = [503, 529];

/// The notice text of the refusal that `line` records: `<kind>: <status> · <message> · <code>`, with the parts the record
/// has, or `None` when `line` is not a refusal record of a known shape (the caller then shows the line itself).
#[must_use]
pub fn describe(line: &str) -> Option<String> {
    // A line that is not JSON is no record: the caller shows it as it is.
    let record: Value = serde_json::from_str(line).ok()?;
    match record.get("type")?.as_str()? {
        "error" => describe_error(&record),
        "result" => describe_result(&record),
        "rate_limit_event" => describe_rate_limit_event(&record),
        _ => None,
    }
}

/// An opencode `error` record, with the provider's fields under `error`, or a codex `error` with a `message`.
fn describe_error(record: &Value) -> Option<String> {
    let Some(error) = record.get("error") else {
        // codex: the message alone, its kind from the words of the message.
        let message = text(record, "message")?;
        return compose(kind_of_words(message), vec![message.to_owned()]);
    };
    let error_type = text(error, "type").unwrap_or_default();
    let status = error.get("status").and_then(Value::as_u64);
    let message = text(error, "message");
    let code = error
        .pointer("/response/body")
        .and_then(Value::as_str)
        .and_then(provider_code);
    let kind = kind_of(error_type, status, code.as_deref());
    let parts = [
        status.map(|status| status.to_string()),
        message.map(str::to_owned),
        code,
    ]
    .into_iter()
    .flatten()
    .collect();
    compose(kind, parts)
}

/// A Claude Code error `result`: the status the API gave and the text of the result.
fn describe_result(record: &Value) -> Option<String> {
    if record.get("is_error").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let status = record.get("api_error_status").and_then(Value::as_u64);
    let result = text(record, "result");
    let kind = kind_of("", status, None).or_else(|| result.and_then(kind_of_words));
    let parts = [status.map(|status| status.to_string()), result.map(str::to_owned)]
        .into_iter()
        .flatten()
        .collect();
    compose(kind, parts)
}

/// A Claude Code `rate_limit_event` that rejected a request: its status and the window it is about.
fn describe_rate_limit_event(record: &Value) -> Option<String> {
    let info = record.get("rate_limit_info")?;
    let status = text(info, "status")?;
    if status != "rejected" {
        return None;
    }
    let window = text(info, "rateLimitType").map(str::to_owned);
    compose(
        Some(Kind::RateLimit),
        [Some(status.to_owned()), window].into_iter().flatten().collect(),
    )
}

/// The string under `key` of `value`, when there is one.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The provider's code of a response body that is JSON; a body that is not JSON has none.
fn provider_code(body: &str) -> Option<String> {
    // A body that is not JSON carries no code: the message of the record is then shown alone.
    let body: Value = serde_json::from_str(body).ok()?;
    body.pointer("/error/code").and_then(Value::as_str).map(str::to_owned)
}

/// The kind of a refusal from the fields of the provider's error: a quota first, then an overload, then a rate limit.
fn kind_of(error_type: &str, status: Option<u64>, code: Option<&str>) -> Option<Kind> {
    if error_type.contains("quota") || status == Some(PAYMENT_REQUIRED) {
        return Some(Kind::Quota);
    }
    let overloaded_code = code.is_some_and(|code| code.contains("overload"));
    if overloaded_code || status.is_some_and(|status| OVERLOADED_STATUSES.contains(&status)) {
        return Some(Kind::Overload);
    }
    if error_type.contains("rate") || status == Some(TOO_MANY_REQUESTS) {
        return Some(Kind::RateLimit);
    }
    None
}

/// The kind a message of a CLI without structured fields names in its words.
fn kind_of_words(message: &str) -> Option<Kind> {
    let lower = message.to_lowercase();
    if lower.contains("capacity") || lower.contains("overload") {
        Some(Kind::Overload)
    } else if lower.contains("rate limit") {
        Some(Kind::RateLimit)
    } else if lower.contains("quota") || lower.contains("usage limit") {
        Some(Kind::Quota)
    } else {
        None
    }
}

/// Joins the parts of a description, with its kind in front when the kind is known; no parts, no description.
fn compose(kind: Option<Kind>, parts: Vec<String>) -> Option<String> {
    if parts.is_empty() {
        return None;
    }
    let body = parts.join(SEPARATOR);
    Some(match kind {
        Some(kind) => format!("{}: {body}", kind.words()),
        None => body,
    })
}
