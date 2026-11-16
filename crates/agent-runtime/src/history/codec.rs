//! How values are kept in the columns: times as fixed-width UTC text, lists as JSON.

use agent_core::state::{InvalidTimestamp, Timestamp};

/// A time as the database keeps it: UTC, RFC 3339, always with nine digits of a second, so that the text sorts as the
/// time does and reads back as the same [`Timestamp`] to the nanosecond: `2026-11-14T08:13:10.000000000Z`.
pub(super) fn time_text(at: Timestamp) -> String {
    let utc = at.as_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second(),
        utc.nanosecond(),
    )
}

/// The time a column holds.
///
/// # Errors
/// [`InvalidTimestamp`] when the text is not an RFC 3339 time.
pub(super) fn parse_time(text: &str) -> Result<Timestamp, InvalidTimestamp> {
    text.parse()
}

/// Normalizes the variable fractional precision of a persisted timestamp for nanosecond-ordered comparisons.
pub(super) fn time_key(at: Timestamp) -> String {
    let text = at.to_string();
    let utc = text.trim_end_matches('Z');
    let (seconds, fraction) = utc.split_once('.').unwrap_or((utc, ""));
    format!("{seconds}.{fraction:0<9}")
}

/// SQL expression normalizing `column` to nanosecond precision (`YYYY-MM-DDTHH:MM:SS.fffffffff`) so text order is
/// timestamp order and matches `daemon_logs_by_time`.
macro_rules! time_key_sql {
    ($column:literal) => {
        concat!(
            "substr(",
            $column,
            ",1,19)||'.'||substr(CASE WHEN substr(",
            $column,
            ",20,1)='.' THEN substr(",
            $column,
            ",21,length(",
            $column,
            ")-21) ELSE '' END||'000000000',1,9)"
        )
    };
}

pub(super) use time_key_sql;
