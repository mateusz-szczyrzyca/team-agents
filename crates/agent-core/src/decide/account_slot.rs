//! Whether an agent of an account may start a run now. An account with `max_runs` has at most that many runs going at
//! once, over all its agents and all teams; a queued task whose account is at its limit waits for one of those runs to
//! end. The waiting task is queued, never failed. An agent without an account, and an account without `max_runs`, have
//! no such limit.

use std::fmt;
use std::num::NonZeroU32;

use super::slot::Going;
use crate::config::{Account, AccountName, Config};
use crate::ids::AgentName;

#[cfg(test)]
mod tests;

/// Whether the account of an agent has a slot for another run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountSlot {
    /// The agent has no account, the account has no limit, or it is below its limit: the run may start.
    Open,
    /// The account is at its limit: the task stays queued until one of its runs ends.
    Full(AccountFull),
}

/// An account at its limit of runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFull {
    /// The account.
    pub account: AccountName,
    /// How many runs of its agents go.
    pub running: u32,
    /// The most runs the account may have going: its `max_runs`.
    pub limit: NonZeroU32,
}

impl fmt::Display for AccountFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "waiting for account {} ({}/{} runs)",
            self.account, self.running, self.limit
        )
    }
}

/// The slot of `account` with `max_runs` as its limit when `running` of its runs go: full when the count has reached the
/// limit; open for an agent without an account and for an account without a limit.
#[must_use]
pub fn account_slot(account: Option<&AccountName>, max_runs: Option<NonZeroU32>, running: u32) -> AccountSlot {
    match (account, max_runs) {
        (Some(account), Some(limit)) if running >= limit.get() => AccountSlot::Full(AccountFull {
            account: account.clone(),
            running,
            limit,
        }),
        _ => AccountSlot::Open,
    }
}

/// The slot an agent `agent` has in the configuration `config` while the runs `going` go, over every team: the runs of
/// every agent and investigator that has the same account are counted.
#[must_use]
pub fn slot_of(config: &Config, agent: &AgentName, going: &[Going]) -> AccountSlot {
    let Some(account) = config.account_of(agent) else {
        return AccountSlot::Open;
    };
    let running = going
        .iter()
        .filter(|run| config.account_of(&run.agent) == Some(account))
        .count();
    // More runs than a `u32` holds cannot go at once; the saturated count is still above any limit.
    let running = u32::try_from(running).unwrap_or(u32::MAX);
    let max_runs = config.accounts().get(account).and_then(Account::max_runs);
    account_slot(Some(account), max_runs, running)
}
