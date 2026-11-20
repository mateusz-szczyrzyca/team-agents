//! A task started again in a new session: what the state keeps of it, and how it reads a state written before it.

use serde::{Deserialize, Serialize};

use super::run::{Run, RunEnd, RunState};
use super::values::Timestamp;
use crate::decide::ladder::Trouble;

/// Renewals of one task, recorded by [`Member::renew`](super::Member::renew): how many times it was started in a new
/// session after its runs went bad, and when the last renewal happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renewals {
    /// How many times the task was started in a new session again.
    used: u32,
    /// When the last renewal happened: the runs that started at or after this time belong to the new session, the
    /// session the task had before it is not resumed any more.
    at: Timestamp,
}

impl Renewals {
    /// How many times the task was started in a new session again.
    #[must_use]
    pub const fn used(&self) -> u32 {
        self.used
    }

    /// When the last renewal happened.
    #[must_use]
    pub const fn at(&self) -> Timestamp {
        self.at
    }

    /// The renewals of a task started anew for the first time, at `at`.
    pub(super) const fn first(at: Timestamp) -> Self {
        Self { used: 1, at }
    }

    /// The renewal the member records: one more was used, at `at`.
    pub(super) fn record(&mut self, at: Timestamp) {
        self.used = self.used.saturating_add(1);
        self.at = at;
    }
}

/// The runs from the first that started at or after `since`, or all of them when `since` is none: the run that carries
/// a renewal starts at its time and belongs to the new session.
pub(super) fn runs_since(runs: &[Run], since: Option<Timestamp>) -> &[Run] {
    match since {
        None => runs,
        Some(since) => {
            let first = runs
                .iter()
                .position(|run| run.started_at().as_utc() >= since.as_utc())
                .unwrap_or(runs.len());
            runs.get(first..).unwrap_or(&[])
        }
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

/// Why a renewal was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenewalError {
    /// The renewal is not after the task's last run: it is of a state that was changed by hand or by a mistake.
    #[error("a renewal at {at} does not follow the run that started at {started_at}")]
    NotAfterTheLastRun {
        /// When the renewal was asked for.
        at: Timestamp,
        /// When the task's last run started.
        started_at: Timestamp,
    },
}

/// Checks that the renewal of `renewals` follows the last run of `runs`: a task is started anew after a run went bad,
/// not between two runs of one session.
pub(super) fn check_renewals(renewals: &Renewals, runs: &[Run]) -> Result<(), RenewalError> {
    let at = renewals.at();
    match runs.last() {
        Some(last) if last.started_at().as_utc() >= at.as_utc() => Ok(()),
        Some(last) => Err(RenewalError::NotAfterTheLastRun {
            at,
            started_at: last.started_at(),
        }),
        None => Ok(()),
    }
}

/// The trouble whose retries the ladder decides from the run of a task that ended.
pub(super) fn trouble_of(end: &RunEnd) -> Option<Trouble> {
    match end {
        RunEnd::NoStatus { .. } => Some(Trouble::NoStatus),
        RunEnd::Hung => Some(Trouble::Hang),
        RunEnd::TurnLimit => Some(Trouble::TurnLimit),
        RunEnd::Reported { .. } | RunEnd::QuotaExhausted { .. } | RunEnd::RateLimited | RunEnd::Stopped => None,
    }
}

/// How many runs since the last renewal ended with `trouble` in a row.
pub(super) fn bad_runs_in_a_row(runs: &[Run], since: Option<Timestamp>, trouble: Trouble) -> u32 {
    let count = runs_since(runs, since)
        .iter()
        .rev()
        .take_while(|run| match run.state() {
            RunState::Ended { end, .. } => trouble_of(end) == Some(trouble),
            RunState::Running => false,
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}
