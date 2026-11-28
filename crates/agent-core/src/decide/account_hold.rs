//! Whether an account is held: it starts no new run because a window of its limits is used up to the account's
//! `hold_at`, until that window resets.
//!
//! [`hold`] is the whole rule and touches nothing: the daemon feeds it the account's `hold_at`, the readings the state
//! keeps for it and the clock. [`held_until`] adds the holds the daemon recorded in the state when a hold began: a later
//! reading below `hold_at` does not end such a hold before its window resets, and a restart of the daemon keeps it.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{AccountName, Config, HoldAt};
use crate::ids::InvestigatorName;
use crate::limit_reading::LimitWindow;
use crate::state::{KeptReading, ReadingOwner, State, Timestamp};

#[cfg(test)]
mod tests;

/// Whether new runs of an account may start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hold {
    /// No kept reading of the account is at its `hold_at`: new runs start.
    Open,
    /// A window is used up to `hold_at`: no new run of the account starts until `until`; the runs going finish.
    Held {
        /// The window whose reset ends the hold: the one that resets last of those at `hold_at`.
        window: LimitWindow,
        /// The share of that window used when it was read.
        used: f64,
        /// When the hold ends: the reset of `window`.
        until: Timestamp,
    },
}

/// Whether an account with `hold_at` is held at `now`, given the `readings` kept for it.
///
/// A reading counts when its window has not reset by `now` and its used share is `hold_at` or more. An account without
/// `hold_at` is never held. With two windows at `hold_at`, the hold lasts until both have reset.
#[must_use]
pub fn hold(hold_at: Option<HoldAt>, readings: &[KeptReading], now: Timestamp) -> Hold {
    let Some(hold_at) = hold_at else {
        return Hold::Open;
    };
    readings
        .iter()
        .filter(|kept| !kept.is_over(now) && kept.reading().used() >= hold_at.share())
        .max_by_key(|kept| kept.reading().resets_at())
        .map_or(Hold::Open, |kept| Hold::Held {
            window: kept.reading().window(),
            used: kept.reading().used(),
            until: kept.reading().resets_at(),
        })
}

/// The accounts of `config` that are held at `now`, with their hold: those whose `hold_at` is reached by a reading the
/// state keeps for them. An account that is not held is not listed.
#[must_use]
pub fn held_accounts(state: &State, config: &Config, now: Timestamp) -> BTreeMap<AccountName, Hold> {
    state
        .limit_readings()
        .live(now)
        .into_iter()
        .filter_map(|(owner, readings)| match owner {
            ReadingOwner::Account(account) => {
                let hold_at = config.accounts().get(account)?.hold_at();
                match hold(hold_at, &readings, now) {
                    Hold::Open => None,
                    held @ Hold::Held { .. } => Some((account.clone(), held)),
                }
            }
            // An agent without an account has no `hold_at`: only accounts are held.
            ReadingOwner::Agent(_) => None,
        })
        .collect()
}

/// The accounts of `config` that are held at `now` and until when: those a recorded hold of the state keeps until its
/// end, and those a kept reading holds; an account in both is held until the later of the two ends.
///
/// A recorded hold counts only while its account has a `hold_at` in `config`, so removing the key lets go of the account.
#[must_use]
pub fn held_until(state: &State, config: &Config, now: Timestamp) -> BTreeMap<AccountName, Timestamp> {
    let mut held: BTreeMap<AccountName, Timestamp> = state
        .account_holds()
        .filter(|(account, until)| {
            now < *until
                && config
                    .accounts()
                    .get(*account)
                    .is_some_and(|account| account.hold_at().is_some())
        })
        .map(|(account, until)| (account.clone(), until))
        .collect();
    for (account, hold) in held_accounts(state, config, now) {
        if let Hold::Held { until, .. } = hold {
            let end = held.entry(account).or_insert(until);
            *end = (*end).max(until);
        }
    }
    held
}

/// The investigators of `config` whose account is held at `now`.
#[must_use]
pub fn investigators_of_held_accounts(state: &State, config: &Config, now: Timestamp) -> BTreeSet<InvestigatorName> {
    let held = held_until(state, config, now);
    config
        .investigators()
        .iter()
        .filter(|(_, investigator)| {
            investigator
                .account
                .as_ref()
                .is_some_and(|account| held.contains_key(account))
        })
        .map(|(name, _)| name.clone())
        .collect()
}
