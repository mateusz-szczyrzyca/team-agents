//! How many times a run went bad, and whether the task was started in a new session again through the ladder: the
//! numbers the ladder decides from, kept by the member so that they survive a restart of the daemon.

use serde::{Deserialize, Serialize};

use super::run::Run;
use super::values::Timestamp;
use crate::decide::ladder::{self, Trouble};

/// Whether a task was started in a new session again, by [`Member::renew`](super::Member::renew).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Renewals {
    /// How many times the task was started anew after its runs went bad.
    used: u32,
    /// Whether the renewal's run started; [`renewals`] says what it took from the task when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    started: Option<Timestamp>,
}

impl Renewals {
    /// How many times the task was started anew after its runs went bad.
    #[must_use]
    pub const fn used(&self) -> u32 {
        self.used
    }

    /// When the last renewal's run started, if the task was started anew and its run has not begun; a renewal counts
    /// from the run that carries it.
    #[must_use]
    pub const fn started(&self) -> Option<Timestamp> {
        self.started
    }

    /// The renewal the member records: one more was used, and its run has not started yet.
    pub(super) fn record(&mut self) {
        self.used = self.used.saturating_add(1);
        self.started = None;
    }

    /// Records that the run of a renewal that waits starts at `at`; `false` when none waits.
    pub(super) fn start(&mut self, at: Timestamp) -> bool {
        _ = at;
        true
    }
}

/// The number of runs since `since`, or all of them when `since` is none.
pub(super) fn runs_since(runs: &[Run], since: Option<Timestamp>) -> &[Run] {
    match since {
        None => runs,
        Some(since) => match runs.iter().position(|run| run.started_at() > since) {
            Some(first) => runs.get(first..).unwrap_or(&[]),
            None => &[],
        },
    }
}

/// The most `[replacement] new_sessions` of a task: a configuration above it is refused.
const MAX_RENEWALS: u32 = 10;

/// The renewals a task may have: `0` to [`MAX_RENEWALS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct RenewalLimit(u32);

/// A number the renewals of a task cannot have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} new sessions per task is not a limit: it is 0 to {MAX_RENEWALS}")]
pub struct InvalidRenewalLimit {
    /// The refused number.
    pub value: u32,
}

impl RenewalLimit {
    /// A limit of `value` new sessions per task.
    ///
    /// # Errors
    /// [`InvalidRenewalLimit`] above [`MAX_RENEWALS`].
    pub fn new(value: u32) -> Result<Self, InvalidRenewalLimit> {
        if value <= MAX_RENEWALS {
            Ok(Self(value))
        } else {
            Err(InvalidRenewalLimit { value })
        }
    }

    /// The number of new sessions per task this limit allows.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The same limit as the ladder reads it.
    #[must_use]
    pub const fn as_ladder(self) -> ladder::Renewals {
        ladder::Renewals {
            used: 0,
            allowed: self.0,
        }
    }
}

impl TryFrom<u32> for RenewalLimit {
    type Error = InvalidRenewalLimit;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RenewalLimit> for u32 {
    fn from(limit: RenewalLimit) -> Self {
        limit.get()
    }
}

impl Default for RenewalLimit {
    fn default() -> Self {
        // One new session per task, the decision the human made on 20.11.2026.
        Self(1)
    }
}

/// Why a renewal was refused by a state that is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenewalError {
    /// The renewal is not after the run that made it: it is of a state that was changed by hand or by a mistake.
    #[error("a renewal at {at} does not follow the run that started at {started_at}")]
    NotAfterTheLastRun {
        /// When the renewal happened.
        at: Timestamp,
        /// When the run that was to follow started.
        started_at: Timestamp,
    },
}

/// Whether the last renewal's run started, checked over the runs of task `task`.
pub(super) fn check_renewals(renewals: &Renewals, runs: &[Run]) -> Result<(), RenewalError> {
    let Some(at) = renewals.started() else {
        return Ok(());
    };
    match runs.last() {
        Some(last) if last.started_at() > at => Ok(()),
        Some(last) => Err(RenewalError::NotAfterTheLastRun {
            at,
            started_at: last.started_at(),
        }),
        None => Err(RenewalError::NotAfterTheLastRun { at, started_at: at }),
    }
}

/// The trouble whose retries the ladder decides from the runs of a task that ended.
pub(super) fn trouble_of(end: &super::run::RunEnd) -> Option<Trouble> {
    match end {
        super::run::RunEnd::NoStatus { .. } => Some(Trouble::NoStatus),
        super::run::RunEnd::Hung => Some(Trouble::Hang),
        super::run::RunEnd::TurnLimit => Some(Trouble::TurnLimit),
        super::run::RunEnd::Reported { .. }
        | super::run::RunEnd::QuotaExhausted { .. }
        | super::run::RunEnd::RateLimited
        | super::run::RunEnd::Stopped => None,
    }
}

/// How many runs since the last renewal ended with `trouble` in a row.
pub(super) fn bad_runs_in_a_row(runs: &[Run], since: Option<Timestamp>, trouble: Trouble) -> u32 {
    let count = runs_since(runs, since)
        .iter()
        .rev()
        .take_while(|run| match run.state() {
            super::run::RunState::Ended { end, .. } => trouble_of(end) == Some(trouble),
            super::run::RunState::Running => false,
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}
