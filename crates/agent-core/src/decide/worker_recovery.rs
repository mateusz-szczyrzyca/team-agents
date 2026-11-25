//! Recovery of a request with no result, independently of wrong-result tries.

use serde::{Deserialize, Serialize};

use super::worker_result::Elsewhere;

/// Maximum returns to the queue without a result; prevents endless provider failures.
pub const MAX_NO_RESULT_RETURNS: u8 = 3;

/// Why an attempt supplied no result; retained even when another attempt succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// The worker process died.
    Died,
    /// The provider refused the request.
    Refused,
    /// The provider exhausted its quota.
    Quota,
    /// The run supplied no terminal status.
    NoStatus,
    /// The daemon stopped before a queued request started.
    RestartQueued,
    /// The daemon stopped during the worker's run.
    RestartRunning,
}

impl Cause {
    /// Stable journal name used in recovery notices.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Died => "died",
            Self::Refused => "refused",
            Self::Quota => "quota",
            Self::NoStatus => "no_status",
            Self::RestartQueued => "restart_queued",
            Self::RestartRunning => "restart_running",
        }
    }
}

/// What happened to the attempt whose ownership has already been checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// No result exists to check.
    NoResult(Cause),
    /// A result exists, including FAILED or QUESTIONS after work: use the result policy.
    Result,
    /// An answer belongs to an older attempt or an already settled request.
    StaleAnswer,
}

/// Ordering used when taking a worker for a queued recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    /// Take the first compatible free worker in configured order.
    FirstFree,
    /// Prefer another provider, falling back to the same provider when necessary.
    OtherProvider,
}

/// What the harness should do, without filesystem or process effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Requeue and immediately serve with the given ordering.
    Serve(Preference),
    /// Requeue until a compatible worker is free, retaining the ordering.
    Wait(Preference),
    /// The no-result bound is reached; return all retained causes to the requester.
    Return,
    /// Check the result under the existing wrong-result try policy.
    Settle,
    /// Drop and log a stale answer without notifying the requester.
    Drop,
}

/// Chooses recovery without spending a wrong-result try.
///
/// `returns` counts completed no-result returns before this event. Provider failures
/// prefer another provider; other interruptions take the first free worker.
#[must_use]
pub fn decide(event: Event, returns: u8, elsewhere: Elsewhere) -> Decision {
    let cause = match event {
        Event::Result => return Decision::Settle,
        Event::StaleAnswer => return Decision::Drop,
        Event::NoResult(cause) => cause,
    };
    if returns >= MAX_NO_RESULT_RETURNS {
        return Decision::Return;
    }
    let preference = match cause {
        Cause::Refused | Cause::Quota => Preference::OtherProvider,
        Cause::Died | Cause::NoStatus | Cause::RestartQueued | Cause::RestartRunning => Preference::FirstFree,
    };
    match elsewhere {
        Elsewhere::Free | Elsewhere::SameProvider => Decision::Serve(preference),
        Elsewhere::None => Decision::Wait(preference),
    }
}

#[cfg(test)]
mod tests;
