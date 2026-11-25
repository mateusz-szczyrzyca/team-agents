use super::*;

/// The record of `DeepInfra` of 23.11.2026: `engine_overloaded` behind opencode's `provider.rate-limit`.
const DEEPINFRA: &str = r#"{"type":"error","error":{"type":"provider.rate-limit","message":"Model busy, retry later","status":429,"response":{"body":"{\"error\":{\"code\":\"engine_overloaded\",\"message\":\"Model busy, retry later\"}}"}}}"#;

#[test]
fn provider_records_have_their_expected_descriptions() {
    let cases = [
        (
            "an_overloaded_model_names_its_status_message_and_code",
            DEEPINFRA,
            Some("an overload: 429 · Model busy, retry later · engine_overloaded"),
        ),
        (
            "a_quota_record_names_the_status_and_the_code",
            r#"{"type":"error","error":{"type":"provider.quota","message":"Insufficient credits","status":402,"response":{"body":"{\"error\":{\"code\":\"insufficient_quota\",\"message\":\"Insufficient credits\"}}"}}}"#,
            Some("a quota: 402 · Insufficient credits · insufficient_quota"),
        ),
        (
            "an_upstream_error_of_status_503_is_an_overload_without_a_body",
            r#"{"type":"error","error":{"type":"provider.internal","message":"Upstream error","status":503}}"#,
            Some("an overload: 503 · Upstream error"),
        ),
        (
            "a_provider_body_that_is_not_json_gives_no_code",
            r#"{"type":"error","error":{"type":"provider.rate-limit","message":"Model busy","status":429,"response":{"body":"<html>busy</html>"}}}"#,
            Some("a rate limit: 429 · Model busy"),
        ),
        (
            "a_claude_error_result_names_its_status_and_its_text",
            r#"{"type":"result","is_error":true,"api_error_status":429,"result":"You've hit your limit · resets 3pm"}"#,
            Some("a quota: 429 · You've hit your limit · resets 3pm"),
        ),
        (
            "a_claude_rejected_rate_limit_event_names_its_window",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1763000000,"rateLimitType":"five_hour"}}"#,
            Some("a quota: rejected · five_hour"),
        ),
        (
            "a_codex_capacity_error_is_an_overload_with_its_message",
            r#"{"type":"error","message":"Selected model is at capacity. Please try a different model."}"#,
            Some("an overload: Selected model is at capacity. Please try a different model."),
        ),
        (
            "a_line_that_is_not_json_is_not_described",
            "429 Too Many Requests",
            None,
        ),
        (
            "a_record_of_another_shape_is_not_described",
            r#"{"type":"text","text":"hello"}"#,
            None,
        ),
    ];
    for (name, line, expected) in cases {
        assert_eq!(describe(line).as_deref(), expected, "{name}");
    }
}

#[test]
fn the_kind_of_evidence_is_the_kind_describe_wrote() {
    let overload = describe(DEEPINFRA).unwrap();
    assert_eq!(kind_of_evidence(&overload), Some(Kind::Overload));
    assert_eq!(
        kind_of_evidence("a quota: 402 · Insufficient credits · insufficient_quota"),
        Some(Kind::Quota)
    );
    assert_eq!(
        kind_of_evidence("a rate limit: rejected · five_hour"),
        Some(Kind::RateLimit)
    );
}

#[test]
fn evidence_that_names_no_kind_in_its_first_words_has_none() {
    for text in [
        "You've hit your limit · resets 3pm",
        "a rate limitation: x",
        "an overload",
    ] {
        assert_eq!(kind_of_evidence(text), None, "{text}");
    }
}

#[test]
fn a_pause_without_a_kind_in_its_evidence_is_the_kind_of_its_reason() {
    assert_eq!(pause_kind(PauseReason::Quota, "You've hit your limit"), Kind::Quota);
    assert_eq!(
        pause_kind(PauseReason::RateLimit, "API Error: 429 rate limit exceeded"),
        Kind::RateLimit
    );
}

#[test]
fn the_kind_the_evidence_names_wins_over_the_reason_of_the_pause() {
    let evidence = describe(DEEPINFRA).unwrap();
    assert_eq!(pause_kind(PauseReason::RateLimit, &evidence), Kind::Overload);
}
