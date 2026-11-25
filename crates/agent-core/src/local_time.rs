//! The times a person reads: one form, in the zone of the system, with the offset valid at the moment shown.
//!
//! The files, the protocol and `--json` keep UTC (RFC 3339 through [`Timestamp`]'s `Display`); this module is only for
//! what a person reads. A [`Zone`] says where a moment is shown: [`Zone::Utc`] is the RFC 3339 text of the files
//! (`agent log --utc`), [`Zone::Fixed`] a fixed offset, [`Zone::Lookup`] the offset that the system has at each moment.
//! Nothing here reads the clock or the environment: a lookup is a function the caller passes in, so the tests choose the
//! rules of their zones.

#[cfg(test)]
mod tests;

use std::fmt;
use std::str::FromStr;

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcDateTime, UtcOffset};

use crate::state::Timestamp;

/// The seconds of a day, as the offset of a lookup is asked for one day before and after a moment.
const DAY_SECONDS: i64 = 86_400;

/// The text a zone that cannot be found is marked with, so that a pasted time says it is UTC.
const UTC_MARK: &str = "UTC";

/// Where a moment is shown.
#[derive(Clone, Copy, Default)]
pub enum Zone {
    /// UTC in the RFC 3339 form of the files, with `Z`: what `agent log --utc` prints. The default: the files' form.
    #[default]
    Utc,
    /// A fixed offset from UTC, shown as `2026-11-21 17:29:53 +01:00`.
    Fixed(UtcOffset),
    /// The offset the system has at the moment asked for, found by the function passed in. When the function gives
    /// no offset, the moment is shown in UTC and marked as UTC.
    Lookup(fn(OffsetDateTime) -> Option<UtcOffset>),
}

/// The offset that applies at a moment, and whether it is UTC standing in for a zone that cannot be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Resolved {
    /// The offset from UTC.
    offset: UtcOffset,
    /// Whether the offset is UTC because the zone could not be found.
    marked: bool,
}

/// The precision of the clock in a shown moment.
#[derive(Clone, Copy)]
enum Precision {
    /// Hour, minute and second (`HH:MM:SS`).
    Second,
    /// Hour and minute (`HH:MM`).
    Minute,
}

/// Why a typed time cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypedTimeError {
    /// The text is neither RFC 3339 with its zone nor a date and a time without one.
    #[error("not a time: {value:?}; write 2026-11-21 17:29 or 2026-11-21T17:29:53, or RFC 3339 with Z or an offset")]
    NotATime {
        /// The text as it was given.
        value: String,
    },
    /// The time is not a moment of the calendar, or it is beyond the range of the state.
    #[error("{value:?} is not a moment that exists")]
    NoSuchMoment {
        /// The text as it was given.
        value: String,
    },
}

/// A time typed on a command line: its form is checked when it is typed, and it is read in a zone when it is used,
/// since the zone of a time is known only then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedTime {
    /// The text as it was typed.
    text: String,
}

impl FromStr for TypedTime {
    type Err = TypedTimeError;

    /// Checks the form of `text`: it is read in UTC, which tells only whether the text is a time and the calendar has it.
    ///
    /// # Errors
    /// [`TypedTimeError`] when `text` is not a time of either form.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Zone::Utc.parse_typed(text)?;
        Ok(Self { text: text.to_owned() })
    }
}

impl TypedTime {
    /// The moment the typed time means in `zone`, as [`Zone::parse_typed`] reads it.
    ///
    /// # Errors
    /// [`TypedTimeError::NoSuchMoment`] when the zone has no such moment for the time.
    pub fn read_in(&self, zone: Zone) -> Result<Timestamp, TypedTimeError> {
        zone.parse_typed(&self.text)
    }
}

impl Zone {
    /// The moment `at` in this zone as [`Zone::stamp`] writes it, without the seconds: `2026-11-21 17:29 +01:00`.
    /// [`Zone::Utc`] writes the RFC 3339 text of the files, which has its seconds.
    #[must_use]
    pub fn stamp_minute(&self, at: Timestamp) -> String {
        self.written(at, Precision::Minute)
    }

    /// The hour and the minute of the moment `at` in this zone, `21:10`, without the date or the zone: for a line that
    /// shows only the time of the day.
    #[must_use]
    pub fn clock_minute(&self, _at: Timestamp) -> String {
        String::new()
    }

    /// Formats the timestamp that ends a daemon `waiting` line (`..., next try <timestamp>`, `..., owed by the lead since <timestamp>`) in
    /// this zone.
    ///
    /// The socket protocol (`RoundDetail.waiting`) carries the retry moment inside the UTC wait text so that the
    /// protocol schema and `agent status --json` stay byte-for-byte compatible while client commands display their
    /// own local zone.
    #[must_use]
    pub fn stamp_waiting(&self, waiting: &str) -> String {
        // The retry moment and, for a wait that has an owner, the moment it started: each ends the line.
        for marker in [", next try ", " since "] {
            let Some((before, raw)) = waiting.rsplit_once(marker) else {
                continue;
            };
            let Ok(at) = self.parse_typed(raw) else {
                continue;
            };
            return format!("{before}{marker}{}", self.stamp_minute(at));
        }
        waiting.to_owned()
    }

    /// The moment `at` in this zone, written as the person reads it: `2026-11-21 17:29:53 +01:00`. The zone is always
    /// written: its offset, or `UTC` when the zone cannot be found or the offset would leave the four-digit years.
    /// [`Zone::Utc`] writes the RFC 3339 text of the files.
    #[must_use]
    pub fn stamp(&self, at: Timestamp) -> String {
        self.written(at, Precision::Second)
    }

    /// The moment `at` in this zone, with its clock written at `precision`.
    fn written(&self, at: Timestamp, precision: Precision) -> String {
        if matches!(self, Self::Utc) {
            return at.to_string();
        }
        let utc = OffsetDateTime::from(at.as_utc());
        let resolved = self.resolve(at.as_utc());
        let (local, resolved) = match utc
            .checked_to_offset(resolved.offset)
            .filter(|local| (0..=9999).contains(&local.year()))
        {
            Some(local) => (local, resolved),
            None => (
                utc,
                Resolved {
                    offset: UtcOffset::UTC,
                    marked: true,
                },
            ),
        };
        let date = local.date();
        let time = local.time();
        let clock = match precision {
            Precision::Second => format!("{:02}:{:02}:{:02}", time.hour(), time.minute(), time.second()),
            Precision::Minute => format!("{:02}:{:02}", time.hour(), time.minute()),
        };
        format!(
            "{:04}-{:02}-{:02} {clock} {}",
            date.year(),
            u8::from(date.month()),
            date.day(),
            zone_label(resolved),
        )
    }

    /// The moment a typed time means in this zone.
    ///
    /// A text with its zone (`Z`, `+01:00`) is read as written. A date and a time without one, `2026-11-21 17:29` or
    /// `2026-11-21T17:29:53.5`, are read in this zone. In a zone with changes of offset, a time that does not exist
    /// (the hour skipped in spring) is read with the offset before the change, and a time that comes twice (the hour
    /// repeated in autumn) is read at its first occurrence.
    ///
    /// # Errors
    /// [`TypedTimeError::NotATime`] when the text is neither, [`TypedTimeError::NoSuchMoment`] when its date or time
    /// does not exist in the calendar.
    pub fn parse_typed(&self, text: &str) -> Result<Timestamp, TypedTimeError> {
        if let Ok(stamp) = text.parse::<Timestamp>() {
            return Ok(stamp);
        }
        let naive = parse_naive(text)?;
        let as_utc = naive.as_utc();
        let offset = match self {
            Self::Utc => UtcOffset::UTC,
            Self::Fixed(offset) => *offset,
            Self::Lookup(_) => self.offset_for_local(naive),
        };
        let instant = as_utc.checked_sub(time::Duration::seconds(i64::from(offset.whole_seconds())));
        let no_such = || TypedTimeError::NoSuchMoment { value: text.to_owned() };
        let moment = instant.ok_or_else(no_such)?;
        Timestamp::from_utc(moment).or(Err(no_such()))
    }

    /// The offset that the local time `naive` has in this zone, following the rules of [`Zone::parse_typed`].
    fn offset_for_local(&self, naive: PrimitiveDateTime) -> UtcOffset {
        let as_utc = naive.as_utc();
        let before = self
            .resolve(as_utc.saturating_sub(time::Duration::seconds(DAY_SECONDS)))
            .offset;
        let after = self
            .resolve(as_utc.saturating_add(time::Duration::seconds(DAY_SECONDS)))
            .offset;
        let candidates = [before, after];
        // An offset is right for `naive` when the moment it gives has that same offset itself.
        let valid = candidates.into_iter().filter(|candidate| {
            let moment = as_utc.saturating_sub(time::Duration::seconds(i64::from(candidate.whole_seconds())));
            self.resolve(moment).offset == *candidate
        });
        // Two valid offsets: the earlier moment is the one with the larger offset, the first occurrence of the hour.
        valid.max_by_key(|offset| offset.whole_seconds()).unwrap_or(before)
    }

    /// The offset valid at `at`, and whether it is UTC standing in for a zone that cannot be found.
    fn resolve(&self, at: UtcDateTime) -> Resolved {
        match self {
            Self::Utc => Resolved {
                offset: UtcOffset::UTC,
                marked: false,
            },
            Self::Fixed(offset) => Resolved {
                offset: *offset,
                marked: false,
            },
            Self::Lookup(lookup) => match lookup(OffsetDateTime::from(at)) {
                Some(offset) => Resolved { offset, marked: false },
                None => Resolved {
                    offset: UtcOffset::UTC,
                    marked: true,
                },
            },
        }
    }
}

impl From<UtcOffset> for Zone {
    /// A fixed offset from UTC, allowing callers and untouched tests with a [`UtcOffset`] to convert into [`Zone`].
    fn from(offset: UtcOffset) -> Self {
        Self::Fixed(offset)
    }
}

impl fmt::Debug for Zone {
    /// The variant, and the offset of a fixed zone; a lookup function has no text to show.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utc => f.write_str("Zone::Utc"),
            Self::Fixed(offset) => write!(f, "Zone::Fixed({offset})"),
            Self::Lookup(_) => f.write_str("Zone::Lookup"),
        }
    }
}

/// The zone of a shown moment: its offset as `+01:00` (with seconds when it has some), or `UTC` when it is marked.
fn zone_label(resolved: Resolved) -> String {
    if resolved.marked {
        return UTC_MARK.to_owned();
    }
    let seconds = resolved.offset.whole_seconds();
    let (sign, magnitude) = if seconds < 0 { ('-', -seconds) } else { ('+', seconds) };
    let (hours, minutes, rest) = (magnitude / 3600, magnitude % 3600 / 60, magnitude % 60);
    if rest == 0 {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{rest:02}")
    }
}

/// Why a clock of a typed time is not read: the text is not a clock, or it is one that the day does not have.
enum ClockError {
    /// The text is not `HH:MM` or `HH:MM:SS` with fractional seconds.
    NotAClock,
    /// The text is a clock, but its hour, minute or second is out of its range.
    NoSuchClock,
}

/// Reads `YYYY-MM-DD`, optionally followed by `T` or a space and `HH:MM` or `HH:MM:SS` with fractional seconds.
fn parse_naive(text: &str) -> Result<PrimitiveDateTime, TypedTimeError> {
    let not_a_time = || TypedTimeError::NotATime { value: text.to_owned() };
    let no_such = || TypedTimeError::NoSuchMoment { value: text.to_owned() };
    let (date_text, time_text) = match text.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let mut parts = date_text.split('-');
    let (Some(year), Some(month), Some(day), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(not_a_time());
    };
    let (Ok(year), Ok(month), Ok(day)) = (year.parse::<i32>(), month.parse::<u8>(), day.parse::<u8>()) else {
        return Err(not_a_time());
    };
    let Ok(month) = Month::try_from(month) else {
        return Err(no_such());
    };
    let Ok(date) = Date::from_calendar_date(year, month, day) else {
        return Err(no_such());
    };
    let time = match time_text {
        None => Time::MIDNIGHT,
        Some(time_text) => match parse_clock(time_text) {
            Ok(time) => time,
            Err(ClockError::NotAClock) => return Err(not_a_time()),
            Err(ClockError::NoSuchClock) => return Err(no_such()),
        },
    };
    Ok(PrimitiveDateTime::new(date, time))
}

/// Reads `HH:MM` or `HH:MM:SS.fff` as a time of the day.
fn parse_clock(text: &str) -> Result<Time, ClockError> {
    let mut parts = text.split(':');
    let (Some(hour), Some(minute)) = (parts.next(), parts.next()) else {
        return Err(ClockError::NotAClock);
    };
    let (Ok(hour), Ok(minute)) = (hour.parse::<u8>(), minute.parse::<u8>()) else {
        return Err(ClockError::NotAClock);
    };
    let (second, nanos) = match parts.next() {
        None => (0, 0),
        Some(rest) => {
            let (whole, fraction) = rest.split_once('.').unwrap_or((rest, ""));
            let Ok(second) = whole.parse::<u8>() else {
                return Err(ClockError::NotAClock);
            };
            let Some(nanos) = fraction_nanos(fraction) else {
                return Err(ClockError::NotAClock);
            };
            (second, nanos)
        }
    };
    if parts.next().is_some() {
        return Err(ClockError::NotAClock);
    }
    Time::from_hms_nano(hour, minute, second, nanos).or(Err(ClockError::NoSuchClock))
}

/// The nanoseconds of the digits of a fraction after a decimal point, to nine places; `None` when they are not digits.
fn fraction_nanos(fraction: &str) -> Option<u32> {
    if fraction.is_empty() {
        return Some(0);
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let digits: String = fraction.chars().chain(std::iter::repeat('0')).take(9).collect();
    digits.parse().ok()
}
