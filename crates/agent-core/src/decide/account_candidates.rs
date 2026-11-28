//! Which agents may be picked while some accounts are paused: an agent of a paused account is not picked, whatever the
//! choice is for — an investigator for a review, a replacement, a worker for a request.
//!
//! [`candidates_with_accounts`] is the whole rule; [`agents_of_paused_accounts`] and [`investigators_of_paused_accounts`]
//! tell it which agents of the configuration those are, from the pauses the state keeps.

use std::collections::BTreeSet;

use crate::config::{AccountName, Config};
use crate::ids::{AgentName, InvestigatorName};
use crate::state::{State, Timestamp};

#[cfg(test)]
mod tests;

/// An agent that may be picked and the account it runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a, N> {
    /// The agent.
    pub name: &'a N,
    /// The account of the agent; `None` for an agent without one, which no account pause concerns.
    pub account: Option<&'a AccountName>,
}

/// The candidates that may be picked: those of `candidates`, in their order, whose account is not one of `paused`. An
/// agent without an account is always one of them.
#[must_use]
pub fn candidates_with_accounts<'a, N>(candidates: &[Candidate<'a, N>], paused: &BTreeSet<AccountName>) -> Vec<&'a N> {
    candidates
        .iter()
        .filter(|candidate| candidate.account.is_none_or(|account| !paused.contains(account)))
        .map(|candidate| candidate.name)
        .collect()
}

/// The accounts that are paused at `now`: the pause the state keeps for each ends later.
#[must_use]
pub fn paused_accounts(state: &State, now: Timestamp) -> BTreeSet<AccountName> {
    state
        .account_pauses()
        .filter(|(_, pause)| now < pause.until)
        .map(|(account, _)| account.clone())
        .collect()
}

/// The agents of `config` whose account is paused at `now`, in name order.
#[must_use]
pub fn agents_of_paused_accounts(state: &State, config: &Config, now: Timestamp) -> Vec<AgentName> {
    let paused = paused_accounts(state, now);
    if paused.is_empty() {
        return Vec::new();
    }
    config
        .agents()
        .iter()
        .filter(|(_, agent)| agent.account.as_ref().is_some_and(|account| paused.contains(account)))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The investigators of `config` whose account is paused at `now`.
#[must_use]
pub fn investigators_of_paused_accounts(state: &State, config: &Config, now: Timestamp) -> BTreeSet<InvestigatorName> {
    let paused = paused_accounts(state, now);
    config
        .investigators()
        .iter()
        .filter(|(_, investigator)| {
            investigator
                .account
                .as_ref()
                .is_some_and(|account| paused.contains(account))
        })
        .map(|(name, _)| name.clone())
        .collect()
}
