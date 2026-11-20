//! The ladder: how many times a task whose runs go bad is started again, and how.
//!
//! A run that ends without a `STATUS:` line or that the watchdog kills leaves the task where it is a few times; after
//! that the session itself is the trouble — the model goes on writing its tool calls as text, or it hangs on the work it
//! kept in its context — and starting the same session again only fails the same way. [`next_step`] decides which rung
//! of the ladder a task takes after such a run:
//!
//! 1. The same session again, up to the retries of its trouble: [`NO_STATUS_RETRIES`] endings without a status, or the
//!    watchdog's strikes for a hang ([`AfterNoStatus`] and [`AfterHang`] of [`super::outcome`]).
//! 2. A new session of the same agent, up to `[replacement] new_sessions` times per task, with a briefing that says what
//!    the previous sessions did.
//! 3. Give up: the task fails and the human decides. Replacing the agent with another one, and asking the human, are
//!    later rungs of the ladder, which this module does not decide yet.

use crate::config::{Limits, Watchdog};
use crate::kinds::Role;
use crate::state::SessionId;

use super::outcome::{AfterNoStatus, NO_STATUS_RETRIES, after_no_status};

/// Why a run of a task went bad in a way the ladder carries over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// The run ended without a `STATUS:` line.
    NoStatus,
    /// The watchdog killed the run after it showed no activity for too long.
    Hang,
    /// The run exhausted its turn budget.
    TurnLimit,
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
    };
    if retried {
        return LadderStep::SameSession;
    }
    if renewals.allowed == 0 || renewals.used >= renewals.allowed {
        return LadderStep::GiveUp;
    }
    match role {
        Role::Solo | Role::Coordinator | Role::Member => LadderStep::NewSession,
        Role::Worker | Role::Reviewer => LadderStep::GiveUp,
    }
}

#[cfg(test)]
mod tests;
