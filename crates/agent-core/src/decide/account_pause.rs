//! What a run refused by its provider does to its agent's account: the pause of the account, shared by every agent that
//! has it, or nothing for an agent without an account.
//!
//! [`decide`] is the whole rule and touches nothing: the daemon feeds it the refusal, the pause the state keeps for the
//! account and the clock, and carries out the answer. The end of the pause is the rule of every limit
//! ([`crate::decide::outcome::pause_until`]).

use crate::config::{AccountName, Limits};
use crate::decide::limit::Limit;
use crate::decide::outcome::{ClassifyError, PauseReason, pause_until};
use crate::state::{AccountPause, Timestamp};

#[cfg(test)]
mod tests;

/// What the pause of an account says about a refused run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountPauseOutcome {
    /// The agent has no account: its refusals are judged one by one, as before accounts.
    NotAnAccount,
    /// The account was not paused: pause it until `until`.
    Pause {
        /// When the pause ends.
        until: Timestamp,
        /// Whether the human is told: once for the account, by the run that starts its pause.
        notify: bool,
    },
    /// The account is paused already, until the time the state keeps: nothing changes and nobody is told again.
    AlreadyPaused,
    /// The pause the state keeps has ended: the daemon forgets it and decides again as for an account that is not paused.
    Expired,
}

/// What a run of an agent of `account` that the provider refused with `limit` does to the pause of its account.
///
/// `paused` is the pause the state keeps for the account, if any.
///
/// # Errors
/// [`ClassifyError`] when the pause would end after the year 9999 or at a time a timestamp cannot hold, or when the
/// configured pause is not a duration.
pub fn decide(
    limit: &Limit,
    limits: &Limits,
    account: Option<&AccountName>,
    paused: Option<&AccountPause>,
    now: Timestamp,
) -> Result<AccountPauseOutcome, ClassifyError> {
    if account.is_none() {
        return Ok(AccountPauseOutcome::NotAnAccount);
    }
    if let Some(paused) = paused {
        return Ok(if now < paused.until {
            AccountPauseOutcome::AlreadyPaused
        } else {
            AccountPauseOutcome::Expired
        });
    }
    let (reset_in, reason) = match *limit {
        Limit::RateLimit => (None, PauseReason::RateLimit),
        Limit::Quota { reset_in } => (reset_in, PauseReason::Quota),
    };
    let until = pause_until(now, reset_in, reason, limits)?;
    Ok(AccountPauseOutcome::Pause { until, notify: true })
}
