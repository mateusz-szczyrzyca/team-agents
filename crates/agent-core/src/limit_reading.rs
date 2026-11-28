//! A reading of a rate-limit window of an account: how much of the window is used and when it resets.
//!
//! A CLI that reports its limits (Claude Code writes `rate_limit_event` records) gives one reading per window. A
//! reading is checked once, when it is made, so that everything that holds one can rely on `used` being a share
//! between 0 and 1.

use serde::{Deserialize, Serialize};
use time::UtcDateTime;

use crate::state::{InvalidTimestamp, Timestamp};

#[cfg(test)]
mod tests;

/// The rate-limit window a reading is of; written in the state as `five_hour` and `seven_day`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitWindow {
    /// The window of the last five hours.
    FiveHour,
    /// The window of the last seven days.
    SevenDay,
}

/// Why a value is no [`LimitReading`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InvalidLimitReading {
    /// The used share is not a number from 0 to 1.
    #[error("the used share {used} of a limit window is not a number from 0 to 1")]
    UsedOutOfRange {
        /// The refused share.
        used: f64,
    },
    /// The reset time is not a time the harness can keep.
    #[error("{seconds} seconds since the epoch is not a reset time the harness can keep")]
    ResetOutOfRange {
        /// The refused seconds since the Unix epoch.
        seconds: i64,
        /// Why the time is refused.
        source: ResetTimeError,
    },
}

/// Why seconds since the epoch are no [`Timestamp`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResetTimeError {
    /// The calendar has no such time.
    #[error("the calendar has no such time")]
    Calendar(#[source] time::error::ComponentRange),
    /// The time is outside the years a [`Timestamp`] holds.
    #[error("the time is outside the years a timestamp holds")]
    Year(#[source] InvalidTimestamp),
}

/// The state of one window at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LimitReading {
    /// The window.
    window: LimitWindow,
    /// The share of the window that is used, from 0 to 1.
    used: f64,
    /// When the window resets.
    resets_at: Timestamp,
}

// A share is never NaN — every constructor and the deserializer refuse it — so equality is reflexive.
impl Eq for LimitReading {}

impl LimitReading {
    /// A reading of `window`.
    ///
    /// # Errors
    /// [`InvalidLimitReading::UsedOutOfRange`] when `used` is not a number from 0 to 1.
    pub fn new(window: LimitWindow, used: f64, resets_at: Timestamp) -> Result<Self, InvalidLimitReading> {
        Self::check_used(used)?;
        Ok(Self {
            window,
            used,
            resets_at,
        })
    }

    /// A reading of `window` that resets `reset_seconds` seconds after the Unix epoch.
    ///
    /// # Errors
    /// [`InvalidLimitReading::UsedOutOfRange`] when `used` is not a number from 0 to 1,
    /// [`InvalidLimitReading::ResetOutOfRange`] when the seconds are no time a [`Timestamp`] holds.
    pub fn from_unix(window: LimitWindow, used: f64, reset_seconds: i64) -> Result<Self, InvalidLimitReading> {
        // The share is checked first: a reading with both wrong is told by its share.
        Self::check_used(used)?;
        let resets_at = UtcDateTime::from_unix_timestamp(reset_seconds)
            .map_err(ResetTimeError::Calendar)
            .and_then(|time| Timestamp::from_utc(time).map_err(ResetTimeError::Year))
            .map_err(|source| InvalidLimitReading::ResetOutOfRange {
                seconds: reset_seconds,
                source,
            })?;
        Ok(Self {
            window,
            used,
            resets_at,
        })
    }

    /// Checks that `used` is a number from 0 to 1; NaN and the infinities are refused too.
    fn check_used(used: f64) -> Result<(), InvalidLimitReading> {
        if (0.0..=1.0).contains(&used) {
            Ok(())
        } else {
            Err(InvalidLimitReading::UsedOutOfRange { used })
        }
    }

    /// The window the reading is of.
    #[must_use]
    pub const fn window(&self) -> LimitWindow {
        self.window
    }

    /// The used share of the window, from 0 to 1.
    #[must_use]
    pub const fn used(&self) -> f64 {
        self.used
    }

    /// When the window resets.
    #[must_use]
    pub const fn resets_at(&self) -> Timestamp {
        self.resets_at
    }
}
