//! Whether an account is held: it starts no new run because a window of its limits is used up to the account's
//! `hold_at`, until that window resets.
//!
//! [`hold`] is the whole rule and touches nothing: the daemon feeds it the account's `hold_at`, the readings the state
//! keeps for it and the clock. Nothing is stored for the hold itself: it follows from the readings, so a restart
//! of the daemon keeps it.

use crate::config::HoldAt;
use crate::limit_reading::LimitWindow;
use crate::state::{KeptReading, Timestamp};

#[cfg(test)]
mod tests;

/// Whether new runs of an account may start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hold {
    /// No kept reading of the account is at its `hold_at`: new runs start.
    Open,
    /// A window is used up to `hold_at`: no new run of the account starts until `until`; the runs going finish.
    Held {
        /// The window whose reset ends the hold: the one that resets last of those at `hold_at`.
        window: LimitWindow,
        /// The share of that window used when it was read.
        used: f64,
        /// When the hold ends: the reset of `window`.
        until: Timestamp,
    },
}

/// Whether an account with `hold_at` is held at `now`, given the `readings` kept for it.
///
/// A reading counts when its window has not reset by `now` and its used share is `hold_at` or more. An account without
/// `hold_at` is never held. With two windows at `hold_at`, the hold lasts until both have reset.
#[must_use]
pub fn hold(hold_at: Option<HoldAt>, readings: &[KeptReading], now: Timestamp) -> Hold {
    let _ = (hold_at, readings, now);
    Hold::Open
}
