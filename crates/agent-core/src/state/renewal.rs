//! A task started again in a new session: what the state keeps of it, and how it reads a state written before it.

use serde::{Deserialize, Serialize};

use super::run::{Run, RunEnd, RunState};
use super::values::Timestamp;
use crate::decide::ladder::Trouble;
use crate::ids::TaskId;
use crate::kinds::Role;

/// Renewals of one task, recorded by [`State::renew_task`](super::State::renew_task): how many times it was started in a new
/// session after its runs went bad, and when the last renewal happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renewals {
    /// How many times the task was started in a new session of its agent again; a hand-over to another agent begins a
    /// session too, and is not counted here.
    used: u32,
    /// When the last renewal or hand-over happened.
    at: Timestamp,
    /// How many runs the task had when it was last started anew or handed over: the runs after them belong to the new
    /// session, and the session the task had before it is not resumed any more.
    after_runs: usize,
    /// Why the task was last started anew.
    reason: Trouble,
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

    /// Why the task was last started anew.
    #[must_use]
    pub const fn reason(&self) -> Trouble {
        self.reason
    }

    /// Whether a task with `runs` runs has begun no run since it was last started anew.
    pub(super) const fn awaits_run(&self, runs: usize) -> bool {
        runs <= self.after_runs
    }

    /// The renewals of a task started anew for the first time, at `at`, for `reason`, after its first `after_runs` runs.
    pub(super) const fn first(at: Timestamp, reason: Trouble, after_runs: usize) -> Self {
        Self {
            used: 1,
            at,
            reason,
            after_runs,
        }
    }

    /// The session boundary of a task handed over to another agent for the first time at `at`, after `reason` made its
    /// first `after_runs` runs go bad: no new session of the task's agent is counted.
    pub(super) const fn handed_over(at: Timestamp, reason: Trouble, after_runs: usize) -> Self {
        Self {
            used: 0,
            at,
            reason,
            after_runs,
        }
    }

    /// The session boundary a hand-over records at `at`, for `reason`, after the first `after_runs` runs: the count of
    /// new sessions stays.
    pub(super) const fn hand_over(&mut self, at: Timestamp, reason: Trouble, after_runs: usize) {
        self.at = at;
        self.reason = reason;
        self.after_runs = after_runs;
    }

    /// The renewal the member records: one more was used, at `at`, for `reason`, after its first `after_runs` runs.
    pub(super) fn record(&mut self, at: Timestamp, reason: Trouble, after_runs: usize) {
        self.used = self.used.saturating_add(1);
        self.at = at;
        self.reason = reason;
        self.after_runs = after_runs;
    }
}

/// The runs of the session the task was last started anew in, or all of them when `renewals` is none.
pub(super) fn runs_since<'a>(runs: &'a [Run], renewals: Option<&Renewals>) -> &'a [Run] {
    match renewals {
        None => runs,
        Some(renewals) => runs.get(renewals.after_runs..).unwrap_or(&[]),
    }
}

/// The most `[replacement] new_sessions` of a task: a configuration above it is refused.
const MAX_RENEWALS: u32 = 10;

/// The renewals a task may have: `0` to 10.
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
    /// [`InvalidRenewalLimit`] above 10.
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
    /// The renewal is before every run of its task: no run went bad before it.
    #[error("a renewal at {at} is before every run of its task")]
    BeforeEveryRun {
        /// When the renewal says the task was started anew.
        at: Timestamp,
    },
}

/// Why [`State::renew_task`](super::State::renew_task) refused to start a task anew.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenewTaskError {
    /// The task is not in the state.
    #[error("task {task} is not in the state")]
    NoTask {
        /// The task.
        task: TaskId,
    },
    /// The task's role is never started anew: a worker serves one request, a reviewer one review.
    #[error("task {task} is a {role} and is never started in a new session")]
    Role {
        /// The task.
        task: TaskId,
        /// Its role.
        role: Role,
    },
    /// The renewal does not fit the task's runs.
    #[error("task {task} cannot be started anew")]
    Renewal {
        /// The task.
        task: TaskId,
        /// What the renewal refused.
        source: RenewalError,
    },
}

/// Checks that the renewal of `renewals` follows a run of `runs`: a task is started anew after a run of it went bad. A
/// renewal after the last run is the state between the ladder's decision and the run of the new session.
pub(super) fn check_renewals(renewals: &Renewals, runs: &[Run]) -> Result<(), RenewalError> {
    let at = renewals.at();
    match runs.first() {
        Some(first) if first.started_at().as_utc() <= at.as_utc() && renewals.after_runs <= runs.len() => Ok(()),
        Some(_) | None => Err(RenewalError::BeforeEveryRun { at }),
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
pub(super) fn bad_runs_in_a_row(runs: &[Run], renewals: Option<&Renewals>, trouble: Trouble) -> u32 {
    let count = runs_since(runs, renewals)
        .iter()
        .rev()
        .take_while(|run| match run.state() {
            RunState::Ended { end, .. } => trouble_of(end) == Some(trouble),
            RunState::Running => false,
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}
