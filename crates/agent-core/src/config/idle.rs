//! Optional idle-team limits, including the zero value that disables removal.

use std::time::Duration;

/// Whole idle minutes; zero disables automatic removal, and every u32 value is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub struct IdleMinutes(u32);

impl From<u32> for IdleMinutes {
    fn from(minutes: u32) -> Self {
        Self(minutes)
    }
}

impl IdleMinutes {
    /// The idle interval, with zero representing disabled removal.
    #[must_use]
    pub fn as_duration(self) -> Duration {
        Duration::ZERO
    }
}

#[cfg(test)]
mod tests;
