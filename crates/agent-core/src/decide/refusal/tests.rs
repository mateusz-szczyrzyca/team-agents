use super::*;

/// The record of `DeepInfra` of 23.11.2026: `engine_overloaded` behind opencode's `provider.rate-limit`.
const DEEPINFRA: &str = r#"{"type":"error","error":{"type":"provider.rate-limit","message":"Model busy, retry later","status":429,"response":{"body":"{\"error\":{\"code\":\"engine_overloaded\",\"message\":\"Model busy, retry later\"}}"}}}"#;

#[test]
fn an_overloaded_model_names_its_status_message_and_code() {
    assert_eq!(
        describe(DEEPINFRA).as_deref(),
        Some("an overload: 429 · Model busy, retry later · engine_overloaded")
    );
}

#[test]
fn a_quota_record_names_the_status_and_the_code() {
    let line = r#"{"type":"error","error":{"type":"provider.quota","message":"Insufficient credits","status":402,"response":{"body":"{\"error\":{\"code\":\"insufficient_quota\",\"message\":\"Insufficient credits\"}}"}}}"#;

    assert_eq!(
        describe(line).as_deref(),
        Some("a quota: 402 · Insufficient credits · insufficient_quota")
    );
}

#[test]
fn an_upstream_error_of_status_503_is_an_overload_without_a_body() {
    let line = r#"{"type":"error","error":{"type":"provider.internal","message":"Upstream error","status":503}}"#;

    assert_eq!(describe(line).as_deref(), Some("an overload: 503 · Upstream error"));
}

#[test]
fn a_provider_body_that_is_not_json_gives_no_code() {
    let line = r#"{"type":"error","error":{"type":"provider.rate-limit","message":"Model busy","status":429,"response":{"body":"<html>busy</html>"}}}"#;

    assert_eq!(describe(line).as_deref(), Some("a rate limit: 429 · Model busy"));
}

#[test]
fn a_claude_error_result_names_its_status_and_its_text() {
    let line =
        r#"{"type":"result","is_error":true,"api_error_status":429,"result":"You've hit your limit · resets 3pm"}"#;

    assert_eq!(
        describe(line).as_deref(),
        Some("a rate limit: 429 · You've hit your limit · resets 3pm")
    );
}

#[test]
fn a_claude_rejected_rate_limit_event_names_its_window() {
    let line = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1763000000,"rateLimitType":"five_hour"}}"#;

    assert_eq!(describe(line).as_deref(), Some("a rate limit: rejected · five_hour"));
}

#[test]
fn a_codex_capacity_error_is_an_overload_with_its_message() {
    let line = r#"{"type":"error","message":"Selected model is at capacity. Please try a different model."}"#;

    assert_eq!(
        describe(line).as_deref(),
        Some("an overload: Selected model is at capacity. Please try a different model.")
    );
}

#[test]
fn a_line_that_is_not_json_is_not_described() {
    assert_eq!(describe("429 Too Many Requests"), None);
}

#[test]
fn a_record_of_another_shape_is_not_described() {
    assert_eq!(describe(r#"{"type":"text","text":"hello"}"#), None);
}
