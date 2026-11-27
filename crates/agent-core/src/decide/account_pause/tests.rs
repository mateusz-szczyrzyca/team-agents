//! Table tests of the pause of an account: which refused runs start it, how long it lasts and when it is no more.

use time::Duration;

use crate::config::{AccountName, Config};
use crate::decide::limit::Limit;
use crate::decide::outcome::{ClassifyError, MAX_PAUSE, RESET_MARGIN};
use crate::state::{AccountPause, Timestamp};

use super::{AccountPauseOutcome, decide};

/// The `[limits]` of these tests: sixty minutes for a quota without a reset, two for a rate limit.
fn config() -> Config {
    Config::from_toml_str("[limits]\nquota_pause_minutes = 60\nrate_limit_pause_minutes = 2\n").unwrap()
}

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// The moment the refused run ended.
fn now() -> Timestamp {
    at("2026-11-27T10:00:00Z")
}

/// The account of these tests.
fn account() -> AccountName {
    "comp-claude".parse().unwrap()
}

/// What [`decide`] says for `limit` of `account`, with `paused` recorded for it, at [`now`].
fn outcome(
    limit: Limit,
    account: Option<&AccountName>,
    paused: Option<&AccountPause>,
) -> Result<AccountPauseOutcome, ClassifyError> {
    decide(&limit, config().limits(), account, paused, now())
}

/// A pause recorded for the quota until `until`.
fn paused_until(until: &str) -> AccountPause {
    AccountPause {
        until: at(until),
        reason: Limit::Quota { reset_in: None },
    }
}

/// The outcome that starts a pause lasting until `until` and tells the human.
fn pause(until: &str) -> AccountPauseOutcome {
    AccountPauseOutcome::Pause {
        until: at(until),
        notify: true,
    }
}

#[test]
fn quota_with_reset_pauses_account() {
    let limit = Limit::Quota {
        reset_in: Some(Duration::minutes(30)),
    };

    // The reset and the margin: 10:00 + 30 minutes + 1 minute.
    assert_eq!(
        outcome(limit, Some(&account()), None),
        Ok(pause("2026-11-27T10:31:00Z"))
    );
}

#[test]
fn quota_with_reset_beyond_the_longest_pause_pauses_for_the_longest_pause() {
    let limit = Limit::Quota {
        reset_in: Some(MAX_PAUSE + Duration::days(30)),
    };

    // Eight days and the margin: 2026-12-05 10:01.
    assert_eq!(
        outcome(limit, Some(&account()), None),
        Ok(pause("2026-12-05T10:01:00Z"))
    );
    assert_eq!(MAX_PAUSE, Duration::days(8));
    assert_eq!(RESET_MARGIN, Duration::minutes(1));
}

#[test]
fn quota_without_reset_pauses_for_the_configured_pause() {
    let limit = Limit::Quota { reset_in: None };

    // `quota_pause_minutes = 60` and the margin.
    assert_eq!(
        outcome(limit, Some(&account()), None),
        Ok(pause("2026-11-27T11:01:00Z"))
    );
}

#[test]
fn rate_limit_pauses_for_the_short_pause() {
    // `rate_limit_pause_minutes = 2` and the margin.
    assert_eq!(
        outcome(Limit::RateLimit, Some(&account()), None),
        Ok(pause("2026-11-27T10:03:00Z"))
    );
}

#[test]
fn agent_without_account_is_not_an_account() {
    let recorded = paused_until("2026-11-27T12:00:00Z");

    assert_eq!(
        outcome(Limit::RateLimit, None, None),
        Ok(AccountPauseOutcome::NotAnAccount)
    );
    assert_eq!(
        outcome(Limit::RateLimit, None, Some(&recorded)),
        Ok(AccountPauseOutcome::NotAnAccount)
    );
}

#[test]
fn second_exhausted_run_while_paused_is_already_paused() {
    let recorded = paused_until("2026-11-27T12:00:00Z");
    let limit = Limit::Quota {
        reset_in: Some(Duration::hours(5)),
    };

    // The pause keeps its reset: a later run's own reset does not move it, and nobody is told twice.
    assert_eq!(
        outcome(limit, Some(&account()), Some(&recorded)),
        Ok(AccountPauseOutcome::AlreadyPaused)
    );
}

#[test]
fn reset_passed_is_expired() {
    let passed = paused_until("2026-11-27T09:59:59Z");
    let at_the_reset = paused_until("2026-11-27T10:00:00Z");

    assert_eq!(
        outcome(Limit::RateLimit, Some(&account()), Some(&passed)),
        Ok(AccountPauseOutcome::Expired)
    );
    assert_eq!(
        outcome(Limit::RateLimit, Some(&account()), Some(&at_the_reset)),
        Ok(AccountPauseOutcome::Expired)
    );
}

#[test]
fn a_pause_that_ends_in_the_next_second_is_still_running() {
    let recorded = paused_until("2026-11-27T10:00:01Z");

    assert_eq!(
        outcome(Limit::RateLimit, Some(&account()), Some(&recorded)),
        Ok(AccountPauseOutcome::AlreadyPaused)
    );
}

#[test]
fn a_pause_that_would_end_after_the_last_year_is_an_error() {
    let late = at("9999-12-31T23:59:00Z");

    let result = decide(&Limit::RateLimit, config().limits(), Some(&account()), None, late);

    assert_eq!(result, Err(ClassifyError::PauseOverflow { now: late }));
}
