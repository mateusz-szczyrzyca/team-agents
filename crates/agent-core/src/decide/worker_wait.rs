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
pub fn wait_verdict(since: Timestamp, now: Timestamp, wait_seconds: u64) -> WaitVerdict {
    // A clock that went back since `since` shows no wait; `max(0)` makes the difference non-negative, so
    // `unsigned_abs` loses nothing.
    let seconds = (now.as_utc() - since.as_utc()).whole_seconds().max(0).unsigned_abs();
    if wait_seconds > 0 && seconds >= wait_seconds {
        WaitVerdict::Return { seconds }
    } else {
        WaitVerdict::Waits { seconds }
    }
}

#[cfg(test)]
mod tests;
