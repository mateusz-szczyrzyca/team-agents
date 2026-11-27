//! Whether a task's session goes on when the account of its agent is not the one the session began under: a session
//! belongs to its account, so a changed account starts a new session from the briefing.
//!
//! [`session_start`] is the whole rule and touches nothing: the daemon feeds it the account the state recorded for the
//! session and the account of the agent now, and carries out the answer.

use crate::config::AccountName;

#[cfg(test)]
mod tests;

/// How the next run of a task starts, as far as its account is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSession {
    /// The session goes on: the agent has the account the session began under, or none, as before.
    Resume,
    /// The account changed: the run starts a new session, briefed as `agent renew` briefs one.
    New,
}

/// What the next run of a task does with its session, given the account `recorded` for the session (`None` for a session
/// without one, and for one begun before accounts) and the account `now` of the agent that runs it.
#[must_use]
pub fn session_start(recorded: Option<&AccountName>, now: Option<&AccountName>) -> AccountSession {
    if recorded == now {
        AccountSession::Resume
    } else {
        AccountSession::New
    }
}
