use super::starts_anew;

#[test]
fn a_context_above_the_bound_starts_the_session_anew() {
    assert!(starts_anew(Some(150_001), Some(150_000)));
    assert!(starts_anew(Some(438_000), Some(150_000)));
}

#[test]
fn a_context_at_or_below_the_bound_resumes_the_session() {
    assert!(!starts_anew(Some(150_000), Some(150_000)));
    assert!(!starts_anew(Some(42_000), Some(150_000)));
    assert!(!starts_anew(Some(0), Some(150_000)));
}

#[test]
fn a_size_the_cli_did_not_tell_resumes_the_session() {
    assert!(!starts_anew(None, Some(150_000)));
    assert!(!starts_anew(None, None));
}

#[test]
fn a_bound_that_is_off_resumes_the_session_whatever_the_size() {
    assert!(!starts_anew(Some(438_000), None));
}

#[test]
fn an_agent_bound_of_one_token_starts_anew_from_two_tokens() {
    assert!(starts_anew(Some(2), Some(1)));
    assert!(!starts_anew(Some(1), Some(1)));
}
