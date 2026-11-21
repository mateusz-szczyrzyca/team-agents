//! The times a person reads: one form, in the zone of the system, with the offset valid at the moment shown.
//!
//! The files, the protocol and `--json` keep UTC; this module is only for what a person reads.

#[cfg(test)]
mod tests;

use time::{OffsetDateTime, UtcOffset};

use crate::state::Timestamp;

/// Where a moment is shown.
#[derive(Clone, Copy)]
pub enum Zone {
    /// UTC in the RFC 3339 form of the files.
    Utc,
    /// A fixed offset from UTC.
    Fixed(UtcOffset),
    /// The offset the system has at the moment asked for.
    Lookup(fn(OffsetDateTime) -> Option<UtcOffset>),
}

/// Why a typed time cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypedTimeError {
    /// The text is neither RFC 3339 with its zone nor a date and a time without one.
    #[error("not a time: {value:?}")]
    NotATime {
        /// The text as it was given.
        value: String,
    },
    /// The time is not a moment of the calendar.
    #[error("{value:?} is not a moment that exists")]
    NoSuchMoment {
        /// The text as it was given.
        value: String,
    },
}

impl Zone {
    /// Stub: written in a later commit.
    #[must_use]
    pub fn stamp(&self, _at: Timestamp) -> String {
        String::new()
    }

    /// Stub: read in a later commit.
    ///
    /// # Errors
    /// Always, for now.
    pub fn parse_typed(&self, text: &str) -> Result<Timestamp, TypedTimeError> {
        Err(TypedTimeError::NotATime { value: text.to_owned() })
    }
}
