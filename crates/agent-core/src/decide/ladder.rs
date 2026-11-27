//! The ladder: how many times a task whose runs go bad is started again, and how.
//!
//! A run that ends without a `STATUS:` line or that the watchdog kills leaves the task where it is a few times; after
//! that the session itself is the trouble — the model goes on writing its tool calls as text, or it hangs on the work it
//! kept in its context — and starting the same session again only fails the same way. [`next_step`] decides which rung
//! of the ladder a task takes after such a run:
//!
//! 1. The same session again, up to the retries of its trouble:
//!    [`NO_STATUS_RETRIES`](super::outcome::NO_STATUS_RETRIES) endings without a status, the watchdog's strikes for a
//!    hang, or the turn-limit resumes ([`super::outcome::after_no_status`], [`super::outcome::after_hang`]).
//! 2. A new session of the same agent, up to `[replacement] new_sessions` times per task, with a briefing that says what
//!    the previous sessions did.
//! 3. Give up: the task fails and the human decides. Replacing the agent with another one, and asking the human, are
//!    later rungs of the ladder, which this module does not decide yet.

use crate::config::{Limits, Watchdog};
use crate::kinds::Role;
use crate::state::SessionId;

use super::outcome::{AfterNoStatus, after_no_status};

/// Why a run of a task went bad in a way the ladder carries over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trouble {
    /// The run ended without a `STATUS:` line.
    NoStatus,
    /// The watchdog killed the run after it showed no activity for too long.
    Hang,
    /// The run exhausted its turn budget.
    TurnLimit,
    /// The run reported `STATUS: FAILED` without calling any tool step: the CLI did not offer, or the run did not use,
    /// the tools of the bus.
    NoToolCall,
    /// The task waited too long for its agent, which was busy in another task: a free agent took it over. No run went bad.
    Busy,
    /// The daemon went down while the run was going: the run was cut from outside, and no run went bad.
    DaemonDown,
    /// The session outgrew its agent's bound: no run went bad, the session is started anew for its size.
    ContextBound,
    /// The run reported `STATUS: FAILED` after it used the bus's tools: the agent itself gave up on the work. It is never
    /// resumed in its session; it goes to a new session once, then up the rest of the ladder.
    Failed,
    /// The round of the task's item went on longer than `[limits] round_minutes` without its pull request in review: the
    /// daemon cut the run and another agent took the task over. No run went bad.
    RoundOverdue,
    /// The human started the task anew with `agent renew`; no run went bad.
    Human,
    /// A human swapped the task's agent with `agent swap`; no run went bad.
    Swap,
}

/// How many sessions of a task were started anew, and the limit `[replacement] new_sessions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renewals {
    /// How many times a new session of the task's agent was started after this many bad runs; the count is kept in the
    /// state, so it survives a restart of the daemon.
    pub used: u32,
    /// How many new sessions the configuration allows per task; `0` turns the rung off.
    pub allowed: u32,
}

/// What happens to a task after a run of it went bad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LadderStep {
    /// Start the session the task already had again.
    SameSession,
    /// Start a new session of the task's agent, and brief it on what the previous sessions did.
    NewSession,
    /// The ladder is out of rungs: the task fails and the human decides.
    GiveUp,
}

/// The step a task takes after a run that went bad.
///
/// `session` is the session the task started its last run in, absent when it had none.
#[must_use]
pub fn next_step(
    role: Role,
    trouble: Trouble,
    count: u32,
    session: Option<&SessionId>,
    renewals: Renewals,
    watchdog: &Watchdog,
    limits: &Limits,
) -> LadderStep {
    // The first rung is the retry each trouble had before the ladder, decided by the same functions: a new session is
    // only started where the session itself is no longer wanted.
    let retried = match trouble {
        Trouble::NoStatus => after_no_status(role, u64::from(count), session) == AfterNoStatus::Resume,
        // `after_hang` reads only whether there is a session to resume; the one a task has is the one it resumes.
        Trouble::Hang => session.is_some() && count < watchdog.hang_strikes.get(),
        Trouble::TurnLimit => session.is_some() && u64::from(count) <= u64::from(limits.turn_limit_resumes),
        // NoToolCall uses its own retry decision; waiting for an agent and daemon cuts are not bad runs.
        // A declared failure is the agent's own verdict on its work: its session is not resumed, the new session comes
        // from the rung below.
        Trouble::NoToolCall
        | Trouble::Busy
        | Trouble::DaemonDown
        | Trouble::ContextBound
        | Trouble::Failed
        | Trouble::RoundOverdue
        | Trouble::Human
        | Trouble::Swap => false,
    };
    if retried {
        return LadderStep::SameSession;
    }
    if renewals.allowed == 0 || renewals.used >= renewals.allowed {
        return LadderStep::GiveUp;
    }
    match role {
        Role::Solo | Role::Coordinator | Role::Member => LadderStep::NewSession,
        Role::Worker | Role::Reviewer | Role::Investigator => LadderStep::GiveUp,
    }
}

#[cfg(test)]
mod tests;
