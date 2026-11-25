use super::*;

#[test]
fn builds_from_fields_with_quota_kind_for_quota_reason() {
    let summary = RefusalSummary::from_fields("402", "Insufficient credits", "", None, PauseReason::Quota);
    assert_eq!(summary.status, "402");
    assert_eq!(summary.message, "Insufficient credits");
    assert_eq!(summary.code, "");
    assert_eq!(summary.kind, RefusalKind::Quota);
    assert_eq!(summary.resets_at, None);
}

#[test]
fn builds_from_fields_with_overloaded_kind_for_429_with_engine_overloaded_code() {
    let summary = RefusalSummary::from_fields(
        "429",
        "Model busy, retry later",
        "engine_overloaded",
        None,
        PauseReason::RateLimit,
    );
    assert_eq!(summary.status, "429");
    assert_eq!(summary.message, "Model busy, retry later");
    assert_eq!(summary.code, "engine_overloaded");
    assert_eq!(summary.kind, RefusalKind::Overloaded);
}

#[test]
fn builds_from_fields_with_overloaded_kind_for_503_status() {
    let summary = RefusalSummary::from_fields("503", "Service Unavailable", "", None, PauseReason::RateLimit);
    assert_eq!(summary.kind, RefusalKind::Overloaded);
}

#[test]
fn builds_from_fields_with_overloaded_kind_for_529_status() {
    let summary = RefusalSummary::from_fields("529", "Overloaded", "", None, PauseReason::RateLimit);
    assert_eq!(summary.kind, RefusalKind::Overloaded);
}

#[test]
fn builds_from_fields_with_provider_error_kind_for_non_limit_status() {
    let summary = RefusalSummary::from_fields("500", "Internal Server Error", "", None, PauseReason::RateLimit);
    assert_eq!(summary.kind, RefusalKind::ProviderError);
}

#[test]
fn builds_from_fields_with_rate_limit_kind_when_status_is_empty() {
    let summary = RefusalSummary::from_fields("", "at capacity", "", None, PauseReason::RateLimit);
    assert_eq!(summary.status, "");
    assert_eq!(summary.kind, RefusalKind::RateLimit);
}

#[test]
fn builds_from_fields_with_rate_limit_kind_for_429_without_overload_code() {
    let summary = RefusalSummary::from_fields("429", "Too Many Requests", "", None, PauseReason::RateLimit);
    assert_eq!(summary.kind, RefusalKind::RateLimit);
}

#[test]
fn builds_from_fields_with_quota_kind_for_openrouter_402() {
    let summary = RefusalSummary::from_fields("402", "Payment Required", "", None, PauseReason::Quota);
    assert_eq!(summary.status, "402");
    assert_eq!(summary.kind, RefusalKind::Quota);
}

#[test]
fn builds_from_fields_with_provider_error_for_internal_error() {
    let summary = RefusalSummary::from_fields("500", "Internal error upstream", "", None, PauseReason::RateLimit);
    assert_eq!(summary.kind, RefusalKind::ProviderError);
}

#[test]
fn builds_from_fields_with_empty_fields_for_codex_fallback() {
    let summary = RefusalSummary::from_fields(
        "",
        "Selected model is at capacity. Please try a different model.",
        "",
        None,
        PauseReason::RateLimit,
    );
    assert_eq!(summary.status, "");
    assert_eq!(summary.code, "");
    assert_eq!(summary.kind, RefusalKind::RateLimit);
}

#[test]
fn builds_from_fields_with_overloaded_kind_for_at_capacity_code() {
    let summary = RefusalSummary::from_fields(
        "503",
        "Service at capacity",
        "at_capacity",
        None,
        PauseReason::RateLimit,
    );
    assert_eq!(summary.kind, RefusalKind::Overloaded);
}

#[test]
fn builds_from_fields_with_resets_at_from_structured_event() {
    let summary = RefusalSummary::from_fields(
        "429",
        "Rate limit exceeded",
        "",
        Some(1_794_471_800),
        PauseReason::RateLimit,
    );
    assert_eq!(summary.resets_at, Some(1_794_471_800));
    assert_eq!(summary.kind, RefusalKind::RateLimit);
}

#[test]
fn every_refusal_kind_has_its_name() {
    for (kind, name) in [
        (RefusalKind::Quota, "quota"),
        (RefusalKind::RateLimit, "rate_limit"),
        (RefusalKind::Overloaded, "overloaded"),
        (RefusalKind::ProviderError, "provider_error"),
    ] {
        assert_eq!(kind.to_string(), name);
    }
}
