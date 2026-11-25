//! How long a queued request waits for a worker of the pool before it is returned to its requester.
//!
//! The daemon notes when it first saw a request queued; this rule says, from that moment, the present one and the
//! limit of `[workers] wait_seconds`, whether the request still waits or goes back.

use crate::state::Timestamp;

/// What is to become of a request that has been queued for some time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitVerdict {
    /// The request goes on waiting; `seconds` is how long it has waited.
    Waits {
        /// Seconds waited so far; `0` when the clock shows no time since the request was first seen.
        seconds: u64,
    },
    /// The request is returned to its requester; `seconds` is how long it waited.
    Return {
        /// Seconds waited.
        seconds: u64,
    },
}

/// Whether a request first seen queued at `since` is returned at `now`, given `[workers] wait_seconds`.
#[must_use]
pub fn wait_verdict(_since: Timestamp, _now: Timestamp, _wait_seconds: u64) -> WaitVerdict {
    WaitVerdict::Return { seconds: 0 }
}

#[cfg(test)]
mod tests;
