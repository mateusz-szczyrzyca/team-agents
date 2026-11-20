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
//! 3. Give up: the task fails and the human decides. Replacing the agent with another one, and asking the human, are the
//!    limps of a later part of the ladder.

use crate::kinds::Role;

use super::outcome::{
    AfterHang, AfterNoStatus, NO_STATUS_RETRIES, after_hang, after_no_status, after_turn_limit, consecutive_no_status,
};

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
/// # Errors
/// None.
pub fn _stub_next_step(
    _role: Role,
    _trouble: Trouble,
    _count: u32,
    _session: Option<&str>,
    _renewals: Renewals,
    _hang_strikes: u32,
    _turn_limit_resumes: u32,
) -> LadderStep {
    _ = (
        _role,
        _trouble,
        _count,
        _session,
        _renewals,
        _hang_strikes,
        _turn_limit_resumes,
    );
    LadderStep::GiveUp
}

#[cfg(test)]
mod tests;
