//! An error and its causes as one line of text.

/// `error` and each of its causes, joined by `: `, for a log record or a message to the human.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    #[test]
    fn an_error_is_followed_by_its_causes() {
        assert_eq!(chain(&Outer(Inner)), "outer: inner");
    }

    #[test]
    fn an_error_without_a_cause_is_its_own_text() {
        assert_eq!(chain(&Inner), "inner");
    }
}
