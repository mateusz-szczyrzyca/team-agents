//! Table tests of the refusal rule: the pause, the retries and the change of agent.

use time::Duration;

use crate::state::RunEnd;

use super::{Decision, Limit, Refusal, decide, next_refusals};

/// A refusal of a rate limit or an overload, with `refusals` in a row and `retries` retries.
fn rate(refusals: u32, retries: u32) -> Refusal {
    Refusal {
        limit: Limit::RateLimit,
        refusals,
        retries,
        wait_limit: Duration::minutes(60),
    }
}

/// A quota refusal whose named reset is `reset_in` away, with `wait_limit` minutes of waiting allowed.
fn quota(reset_in: Option<Duration>, refusals: u32, wait_limit: i64) -> Refusal {
    Refusal {
        limit: Limit::Quota { reset_in },
        refusals,
        retries: 1,
        wait_limit: Duration::minutes(wait_limit),
    }
}

#[test]
fn first_rate_limit_waits_when_one_retry_is_allowed() {
    assert_eq!(decide(&rate(1, 1)), Decision::Pause);
}

#[test]
fn retry_refused_after_one_retry_changes_agent() {
    assert_eq!(decide(&rate(2, 1)), Decision::ChangeAgent);
}

#[test]
fn zero_retries_change_agent_at_the_first_refusal() {
    assert_eq!(decide(&rate(1, 0)), Decision::ChangeAgent);
}

#[test]
fn three_retries_wait_through_three_refusals_and_change_at_the_fourth() {
    assert_eq!(decide(&rate(1, 3)), Decision::Pause);
    assert_eq!(decide(&rate(2, 3)), Decision::Pause);
    assert_eq!(decide(&rate(3, 3)), Decision::Pause);
    assert_eq!(decide(&rate(4, 3)), Decision::ChangeAgent);
}

#[test]
fn overload_is_a_rate_limit_for_the_rule() {
    // An overload reaches the rule as a rate limit: it has no reset of its own.
    assert_eq!(decide(&rate(2, 1)), Decision::ChangeAgent);
}

#[test]
fn quota_reset_below_the_wait_limit_waits() {
    assert_eq!(decide(&quota(Some(Duration::minutes(30)), 1, 60)), Decision::Pause);
}

#[test]
fn quota_reset_at_the_wait_limit_waits() {
    assert_eq!(decide(&quota(Some(Duration::minutes(60)), 1, 60)), Decision::Pause);
}

#[test]
fn quota_reset_above_the_wait_limit_changes_agent_at_once() {
    assert_eq!(
        decide(&quota(Some(Duration::minutes(61)), 1, 60)),
        Decision::ChangeAgent
    );
}

#[test]
fn zero_wait_limit_never_waits_for_a_named_reset() {
    assert_eq!(decide(&quota(Some(Duration::minutes(1)), 1, 0)), Decision::ChangeAgent);
}

#[test]
fn quota_without_a_named_reset_follows_the_retries() {
    assert_eq!(decide(&quota(None, 1, 0)), Decision::Pause);
    assert_eq!(decide(&quota(None, 2, 0)), Decision::ChangeAgent);
}

#[test]
fn refusals_in_a_row_are_counted_and_a_good_run_clears_them() {
    let first = next_refusals(0, &RunEnd::RateLimited);
    assert_eq!(first, 1);
    let second = next_refusals(first, &RunEnd::RateLimited);
    assert_eq!(second, 2);
    assert_eq!(next_refusals(second, &RunEnd::Stopped), 0);
    assert_eq!(
        decide(&rate(next_refusals(0, &RunEnd::RateLimited), 1)),
        Decision::Pause
    );
}

#[test]
fn the_count_saturates_instead_of_wrapping() {
    assert_eq!(next_refusals(u32::MAX, &RunEnd::RateLimited), u32::MAX);
}
