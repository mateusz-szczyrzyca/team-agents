//! The small values of the state: a point in time, a model CLI's session ID, a commit and a pull request number.
//!
//! Each is made only through its check; its text form (`Display`, serde) parses back to an equal value.

use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::UtcDateTime;
use time::error::Parse;
use time::format_description::well_known::Rfc3339;

use crate::config::{InvalidValue, check_text};
use crate::ids::Clock;

/// Latest year a [`Timestamp`] can have: RFC 3339 writes the year with four digits.
const MAX_YEAR: i32 = 9999;

/// Longest session ID, in bytes: a bound on what a model CLI prints, far above its real IDs.
const MAX_SESSION_LEN: usize = 200;

/// Length of a SHA-1 commit ID in hexadecimal digits.
const SHA1_HEX_LEN: usize = 40;

/// Length of a SHA-256 commit ID in hexadecimal digits.
const SHA256_HEX_LEN: usize = 64;

/// A point in time, in UTC, written in the state as RFC 3339 (`2026-11-11T19:15:54Z`).
///
/// The year is 0000 to 9999. Parsing accepts any UTC offset and converts to UTC; the text form is always UTC with a
/// `Z`, and fractions of a second only when there are some. Timestamps order by time.
///
/// ```
/// # fn main() -> Result<(), agent_core::state::InvalidTimestamp> {
/// let at: agent_core::state::Timestamp = "2026-11-11T20:15:54+01:00".parse()?;
/// assert_eq!(at.to_string(), "2026-11-11T19:15:54Z");
/// # Ok(())
/// # }
/// ```
///
/// The only way to a value is through the checks:
///
/// ```compile_fail,E0603
/// let at = agent_core::state::Timestamp(time::UtcDateTime::UNIX_EPOCH);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(UtcDateTime);

/// Why a text or a time is not a [`Timestamp`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidTimestamp {
    /// The text is not an RFC 3339 date and time.
    #[error("{value:?} is not an RFC 3339 date and time such as 2026-11-11T19:15:54Z")]
    NotRfc3339 {
        /// The refused text.
        value: String,
        /// What the parser found wrong.
        source: Parse,
    },
    /// The year is outside 0000 to 9999.
    #[error("the year {year} is outside 0000 to 9999")]
    YearOutOfRange {
        /// The year of the time.
        year: i32,
    },
}

impl Timestamp {
    /// The time `value`.
    ///
    /// # Errors
    /// [`InvalidTimestamp::YearOutOfRange`] when the year is not 0000 to 9999.
    pub const fn from_utc(value: UtcDateTime) -> Result<Self, InvalidTimestamp> {
        let year = value.year();
        if year < 0 || year > MAX_YEAR {
            return Err(InvalidTimestamp::YearOutOfRange { year });
        }
        Ok(Self(value))
    }

    /// The time `clock` gives now.
    ///
    /// # Errors
    /// [`InvalidTimestamp::YearOutOfRange`] when the clock's year is not 0000 to 9999.
    pub fn now(clock: &impl Clock) -> Result<Self, InvalidTimestamp> {
        Self::from_utc(clock.now())
    }

    /// The time, in UTC.
    #[must_use]
    pub const fn as_utc(self) -> UtcDateTime {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `from_utc` keeps the year in 0000 to 9999, the one thing RFC 3339 formatting refuses; `fmt::Error` has no
        // room for the cause, so a failure that cannot happen here is reported as the formatter's own error.
        let text = self.0.format(&Rfc3339).map_err(|_unreachable| fmt::Error)?;
        f.write_str(&text)
    }
}

impl FromStr for Timestamp {
    type Err = InvalidTimestamp;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let value = UtcDateTime::parse(text, &Rfc3339).map_err(|source| InvalidTimestamp::NotRfc3339 {
            value: text.to_owned(),
            source,
        })?;
        Self::from_utc(value)
    }
}

impl TryFrom<String> for Timestamp {
    type Error = InvalidTimestamp;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Timestamp> for String {
    fn from(value: Timestamp) -> Self {
        value.to_string()
    }
}

impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// The session ID a model CLI gave a run, which a later run resumes: opaque text such as
/// `5f3c9a3e-86f3-4b0e-9d2c-0a8f1c7d5b21`.
///
/// 1 to 200 bytes without whitespace or control characters. Refused, with the [`ValueRule`](crate::config::ValueRule) it breaks: the empty
/// text, a longer one, and any whitespace or control character.
///
/// ```compile_fail,E0603
/// let id = agent_core::state::SessionId(String::from("x"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionId(String);

impl SessionId {
    /// The ID's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SessionId {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        check_session_id(text)?;
        Ok(Self(text.to_owned()))
    }
}

impl TryFrom<String> for SessionId {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        check_session_id(&text)?;
        Ok(Self(text))
    }
}

/// Checks the rules of a [`SessionId`].
fn check_session_id(text: &str) -> Result<(), InvalidValue> {
    check_text("session ID", text, MAX_SESSION_LEN, |c| {
        !c.is_control() && !c.is_whitespace()
    })
}

impl From<SessionId> for String {
    fn from(value: SessionId) -> Self {
        value.0
    }
}

/// A git commit ID: 40 (SHA-1) or 64 (SHA-256) lowercase hexadecimal digits.
///
/// ```compile_fail,E0603
/// let id = agent_core::state::CommitId(String::from("x"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitId(String);

/// A text that is not a [`CommitId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not a commit ID: 40 or 64 lowercase hexadecimal digits")]
pub struct InvalidCommitId {
    /// The refused text.
    pub value: String,
}

impl CommitId {
    /// The ID's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CommitId {
    type Err = InvalidCommitId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        check_commit_id(text)?;
        Ok(Self(text.to_owned()))
    }
}

impl TryFrom<String> for CommitId {
    type Error = InvalidCommitId;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        check_commit_id(&text)?;
        Ok(Self(text))
    }
}

/// Checks the rules of a [`CommitId`].
fn check_commit_id(text: &str) -> Result<(), InvalidCommitId> {
    let digits_only = text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if digits_only && matches!(text.len(), SHA1_HEX_LEN | SHA256_HEX_LEN) {
        Ok(())
    } else {
        Err(InvalidCommitId { value: text.to_owned() })
    }
}

impl From<CommitId> for String {
    fn from(value: CommitId) -> Self {
        value.0
    }
}

/// The number of a pull request in its repository, such as `82`; never 0. Written in the state as a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrNumber(NonZeroU32);

impl PrNumber {
    /// The pull request numbered `number`.
    #[must_use]
    pub const fn new(number: NonZeroU32) -> Self {
        Self(number)
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> NonZeroU32 {
        self.0
    }
}

impl fmt::Display for PrNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::error::Category;
    use time::{Date, Month, Time};

    use super::*;
    use crate::config::ValueRule;

    /// A clock that stands still.
    struct FixedClock(UtcDateTime);

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            self.0
        }
    }

    /// A UTC time from its parts.
    fn utc(year: i32, month: Month, day: u8, hour: u8, minute: u8, second: u8) -> UtcDateTime {
        UtcDateTime::new(
            Date::from_calendar_date(year, month, day).unwrap(),
            Time::from_hms(hour, minute, second).unwrap(),
        )
    }

    /// The time the examples use.
    fn evening() -> UtcDateTime {
        utc(2026, Month::November, 11, 19, 15, 54)
    }

    // ---- Timestamp ----------------------------------------------------------------------------------------------

    #[test]
    fn a_timestamp_is_written_as_rfc_3339_in_utc() {
        let at = Timestamp::from_utc(evening()).unwrap();

        assert_eq!(at.to_string(), "2026-11-11T19:15:54Z");
        assert_eq!(String::from(at), "2026-11-11T19:15:54Z");
        assert_eq!(at.as_utc(), evening());
    }

    #[test]
    fn fractions_of_a_second_are_kept_and_only_written_when_there_are_some() {
        let at = Timestamp::from_utc(evening().replace_nanosecond(500_000_000).unwrap()).unwrap();

        assert_eq!(at.to_string(), "2026-11-11T19:15:54.5Z");
        assert_eq!(at.to_string().parse::<Timestamp>(), Ok(at));
    }

    #[test]
    fn a_timestamp_parses_in_any_offset_and_is_kept_in_utc() {
        for text in [
            "2026-11-11T19:15:54Z",
            "2026-11-11T20:15:54+01:00",
            "2026-11-11T14:15:54-05:00",
            "2026-11-11T19:15:54+00:00",
            "2026-11-12T01:00:54+05:45",
        ] {
            let at: Timestamp = text.parse().unwrap();

            assert_eq!(at.as_utc(), evening(), "{text}");
            assert_eq!(at.to_string(), "2026-11-11T19:15:54Z", "{text}");
        }
    }

    #[test]
    fn a_timestamp_round_trips_through_json_as_a_string() {
        let at = Timestamp::from_utc(evening()).unwrap();

        let json = serde_json::to_string(&at).unwrap();

        assert_eq!(json, "\"2026-11-11T19:15:54Z\"");
        assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), at);
    }

    #[test]
    fn a_text_that_is_not_a_date_and_time_is_refused_with_its_text_and_the_parse_failure() {
        for text in [
            "",
            "yesterday",
            "2026-11-11",
            "2026-11-11T19:15:54",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-11-11T24:00:00Z",
            "2026-11-11T19:60:00Z",
            " 2026-11-11T19:15:54Z",
            "2026-11-11T19:15:54Z ",
            "1700000000",
            "2026-11-11T19:15:54+25:00",
        ] {
            let Err(InvalidTimestamp::NotRfc3339 { value, source }) = text.parse::<Timestamp>() else {
                panic!("{text:?} is accepted");
            };

            assert_eq!(value, text);
            assert_eq!(Err(source), UtcDateTime::parse(text, &Rfc3339).map(|_| ()), "{text:?}");
            let json_err = serde_json::from_str::<Timestamp>(&serde_json::to_string(text).unwrap()).unwrap_err();
            assert_eq!(json_err.classify(), Category::Data, "{text:?}");
        }
    }

    #[test]
    fn a_time_outside_the_years_0_to_9999_is_refused() {
        let early = utc(-1, Month::December, 31, 23, 59, 59);

        assert_eq!(
            Timestamp::from_utc(early),
            Err(InvalidTimestamp::YearOutOfRange { year: -1 })
        );
        Timestamp::from_utc(utc(0, Month::January, 1, 0, 0, 0)).unwrap();
        Timestamp::from_utc(utc(9999, Month::December, 31, 23, 59, 59)).unwrap();
    }

    #[test]
    fn a_timestamp_from_a_clock_is_the_clocks_time() {
        let clock = FixedClock(evening());

        assert_eq!(Timestamp::now(&clock).unwrap().as_utc(), evening());
        assert_eq!(
            Timestamp::now(&FixedClock(utc(-5, Month::May, 5, 5, 5, 5))),
            Err(InvalidTimestamp::YearOutOfRange { year: -5 })
        );
    }

    #[test]
    fn timestamps_order_by_time() {
        let earlier: Timestamp = "2026-11-11T19:15:54Z".parse().unwrap();
        let later: Timestamp = "2026-11-11T19:15:55Z".parse().unwrap();
        let same_as_earlier: Timestamp = "2026-11-11T21:15:54+02:00".parse().unwrap();

        assert!(earlier < later);
        assert_eq!(earlier, same_as_earlier);
    }

    // ---- SessionId ----------------------------------------------------------------------------------------------

    #[test]
    fn session_ids_of_the_model_clis_are_accepted_and_round_trip() {
        for text in [
            "5f3c9a3e-86f3-4b0e-9d2c-0a8f1c7d5b21",
            "ses_3c1f9a2b0ffeAbCdEf",
            "thread/019a-x",
            "x",
            "żółć-1",
        ] {
            let id: SessionId = text.parse().unwrap();

            assert_eq!(id.as_str(), text);
            assert_eq!(id.to_string(), text);
            assert_eq!(String::from(id.clone()), text);
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), id);
        }
    }

    #[test]
    fn a_session_id_has_at_most_200_bytes() {
        let longest = "a".repeat(MAX_SESSION_LEN);
        let too_long = "a".repeat(MAX_SESSION_LEN + 1);

        longest.parse::<SessionId>().unwrap();
        let err = too_long.parse::<SessionId>().unwrap_err();
        assert_eq!(err.what, "session ID");
        assert_eq!(err.value, too_long);
        assert_eq!(err.rule, ValueRule::TooLong { max: MAX_SESSION_LEN });
    }

    #[test]
    fn a_session_id_refuses_the_empty_text_whitespace_and_control_characters() {
        let cases = [
            ("", ValueRule::Empty),
            (" ", ValueRule::ForbiddenChar(' ')),
            ("a b", ValueRule::ForbiddenChar(' ')),
            ("a\tb", ValueRule::ForbiddenChar('\t')),
            ("a\nb", ValueRule::ForbiddenChar('\n')),
            ("a\0b", ValueRule::ForbiddenChar('\0')),
            ("a\u{7f}b", ValueRule::ForbiddenChar('\u{7f}')),
            ("a\u{a0}b", ValueRule::ForbiddenChar('\u{a0}')),
        ];
        for (text, rule) in cases {
            let err = text.parse::<SessionId>().unwrap_err();

            assert_eq!((err.what, err.value.as_str(), err.rule), ("session ID", text, rule));
            let json_err = serde_json::from_str::<SessionId>(&serde_json::to_string(text).unwrap()).unwrap_err();
            assert_eq!(json_err.classify(), Category::Data, "{text:?}");
        }
    }

    // ---- CommitId -----------------------------------------------------------------------------------------------

    #[test]
    fn a_commit_id_is_40_or_64_lowercase_hex_digits() {
        let sha1 = "26b65de7e4076596768ca96b48a7f94799c93874";
        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        for text in [sha1, sha256] {
            let id: CommitId = text.parse().unwrap();

            assert_eq!(id.as_str(), text);
            assert_eq!(id.to_string(), text);
            assert_eq!(CommitId::try_from(text.to_owned()).unwrap(), id);
            assert_eq!(String::from(id.clone()), text);
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(serde_json::from_str::<CommitId>(&json).unwrap(), id);
        }
    }

    #[test]
    fn anything_else_is_not_a_commit_id() {
        let sha1 = "26b65de7e4076596768ca96b48a7f94799c93874";
        let short = "26b65de7e4076596768ca96b48a7f94799c9387";
        for text in [
            "",
            "26b65de",
            short,
            &format!("{sha1}0"),
            &sha1.to_uppercase(),
            &format!("{short}g"),
            &format!(" {short}"),
            &format!("{sha1}\n"),
            &"a".repeat(63),
            &"a".repeat(65),
            &"é".repeat(20),
        ] {
            assert_eq!(
                text.parse::<CommitId>(),
                Err(InvalidCommitId { value: text.to_owned() }),
                "{text:?}"
            );
            let json_err = serde_json::from_str::<CommitId>(&serde_json::to_string(text).unwrap()).unwrap_err();
            assert_eq!(json_err.classify(), Category::Data, "{text:?}");
        }
    }

    // ---- PrNumber -----------------------------------------------------------------------------------------------

    #[test]
    fn a_pull_request_number_is_a_json_number_and_never_zero() {
        let number = PrNumber::new(NonZeroU32::new(82).unwrap());

        assert_eq!(number.get().get(), 82);
        assert_eq!(number.to_string(), "82");
        assert_eq!(serde_json::to_string(&number).unwrap(), "82");
        assert_eq!(serde_json::from_str::<PrNumber>("82").unwrap(), number);
        for bad in ["0", "-1", "\"82\"", "4294967296", "1.5", "null"] {
            let err = serde_json::from_str::<PrNumber>(bad).unwrap_err();
            assert_eq!(err.classify(), Category::Data, "{bad}");
        }
    }
}
