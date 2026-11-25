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
        (
            "codex_turn_failed",
            r#"{"type":"turn.failed","error":{"message":"Selected model is at capacity. Please try a different model."}}"#,
            Some("an overload: Selected model is at capacity. Please try a different model."),
        ),
        (
            "claude_seven_day",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"seven_day"}}"#,
            Some("a quota: rejected · seven_day"),
        ),
        (
            "claude_request_rate",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"requests_per_minute"}}"#,
            Some("a rate limit: rejected · requests_per_minute"),
        ),
    ];
    for (name, line, expected) in cases {
        assert_eq!(describe(line).as_deref(), expected, "{name}");
    }
}

#[test]
fn structured_records_carry_their_kind_without_parsing_evidence() {
    for (line, expected) in [
        (DEEPINFRA, Some(Kind::Overload)),
        (
            r#"{"type":"error","error":{"type":"provider.quota","status":402}}"#,
            Some(Kind::Quota),
        ),
        (
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"requests"}}"#,
            Some(Kind::RateLimit),
        ),
    ] {
        assert_eq!(read(line).and_then(|description| description.kind), expected, "{line}");
    }
}

#[test]
fn plain_text_is_not_a_structured_record() {
    for text in [
        "You've hit your limit · resets 3pm",
        "a rate limitation: x",
        "an overload",
    ] {
        assert_eq!(read(text), None, "{text}");
    }
}

#[test]
fn plain_text_pauses_get_the_kind_of_their_reason() {
    use super::super::outcome::{ExitStatus, LimitContext, RunFinish, StoppedBy, classify_outcome};
    use crate::cli::{ErrorTail, RunOutput, StreamOutcome};
    use crate::config::Config;
    let config = Config::default();
    for (line, expected) in [
        ("You've hit your limit", Kind::Quota),
        ("API Error: 429 rate limit exceeded", Kind::RateLimit),
    ] {
        let output = RunOutput {
            final_message: None,
            session_id: None,
            stream: StreamOutcome::Failed,
            error_tail: ErrorTail::from_text(line, config.limits().tail_lines),
        };
        let classified = classify_outcome(
            &output,
            RunFinish {
                exit: ExitStatus::Code(1),
                stopped_by: StoppedBy::Nobody,
            },
            &LimitContext {
                limits: config.limits(),
                now: "2026-11-12T10:00:00Z".parse().unwrap(),
                local_offset: time::UtcOffset::UTC,
            },
        )
        .unwrap();
        assert_eq!(classified.pause.unwrap().kind, expected, "{line}");
    }
}

#[test]
fn a_structured_overload_carries_its_kind() {
    assert_eq!(read(DEEPINFRA).unwrap().kind, Some(Kind::Overload));
}
