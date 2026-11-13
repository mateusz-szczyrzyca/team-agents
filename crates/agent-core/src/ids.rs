//! Identifiers: names of agents and investigators, IDs of teams, rounds, tasks and runs, task-list item IDs and git
//! branch names.
//!
//! Each is a newtype that can hold only a valid value (TYP-1, TYP-2). A value is made by parsing — [`FromStr`],
//! [`TryFrom<String>`] and serde all run the same check — or, for the IDs the harness creates, by `generate`
//! ([`TeamId::generate`], [`RoundId::generate`], [`TaskId::generate`], [`RunId::generate`]), which takes the clock and
//! the random source as arguments. The text form ([`Display`](fmt::Display), serde) parses back to an equal value.

use std::fmt;
use std::num::{NonZeroU16, ParseIntError};
use std::str::FromStr;

use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};
use time::error::ComponentRange;
use time::{Date, Month, Time, UtcDateTime};

/// Longest agent or investigator name, in bytes: names become parts of branch names, file names and unit names.
pub const MAX_NAME_LEN: usize = 32;

/// Longest branch name, in bytes: a bound on untrusted input, far above the branches the harness creates.
pub const MAX_BRANCH_LEN: usize = 255;

/// Number of random characters at the end of a generated ID.
const SUFFIX_LEN: usize = 4;

/// Number of distinct characters in a generated ID's suffix: the digits `0-9`, then the letters `a-z`.
const SUFFIX_RADIX: u8 = 36;

/// Latest year a generated ID can carry: its timestamp has four digits for the year.
const MAX_YEAR: i32 = 9999;

/// Characters a branch name may not contain besides ASCII control characters (`git check-ref-format`, rules 4, 5
/// and 10).
const BRANCH_FORBIDDEN: &[char] = &[' ', '~', '^', ':', '?', '*', '[', '\\'];

/// Implements the text conversions of a validated string newtype `$ty(String)`: `as_str`, `Display`, `AsRef<str>`,
/// `FromStr` and `TryFrom<String>` (which serde deserializes through), both running `$check`, and `From<$ty> for
/// String` (which serde serializes through).
macro_rules! text_newtype {
    ($ty:ident, $err:ty, $check:expr) => {
        impl $ty {
            /// The value's text, as it was parsed.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $ty {
            type Err = $err;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                $check(text)?;
                Ok(Self(text.to_owned()))
            }
        }

        impl TryFrom<String> for $ty {
            type Error = $err;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                $check(&text)?;
                Ok(Self(text))
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.0
            }
        }
    };
}

/// Implements `generate` for a generated ID `$ty(String)` of kind `$kind`.
macro_rules! generated_id {
    ($ty:ident, $kind:expr) => {
        text_newtype!($ty, InvalidId, |text: &str| check_id($kind, text));

        impl $ty {
            /// Creates a new ID from the time `clock` gives and four characters drawn from `rng`.
            ///
            /// Two IDs made in the same second differ in their suffix with a probability of 1 − 36⁻⁴; whoever stores
            /// them refuses the rare duplicate.
            ///
            /// # Errors
            /// [`GenerateIdError`] when the clock's year is outside 0000–9999.
            pub fn generate(clock: &impl Clock, rng: &mut impl Rng) -> Result<Self, GenerateIdError> {
                generate_id($kind, clock, rng).map(Self)
            }
        }
    };
}

/// Name of a normal or worker agent from the configuration, such as `codex-1`.
///
/// 1 to [`MAX_NAME_LEN`] bytes of lowercase ASCII letters, digits and `-`, starting with a letter and not ending with
/// `-`. Refused, with the [`NameRule`] it breaks: the empty text, a longer one, a first character that is not a
/// lowercase letter, any character other than `a-z`, `0-9` and `-` (uppercase letters, `_`, `.`, `/`, spaces,
/// non-ASCII letters), and a last `-`.
///
/// ```
/// # fn main() -> Result<(), agent_core::ids::InvalidName> {
/// let name: agent_core::ids::AgentName = "codex-1".parse()?;
/// assert_eq!(name.as_str(), "codex-1");
/// # Ok(())
/// # }
/// ```
///
/// The only way to a value is through that check:
///
/// ```compile_fail,E0603
/// let name = agent_core::ids::AgentName(String::from("Not Valid"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentName(String);

text_newtype!(AgentName, InvalidName, |text: &str| check_name(NameKind::Agent, text));

/// Name of an investigator from the configuration, such as `inv-opus`: the same rules as [`AgentName`], a distinct
/// type so that the two lists cannot be mixed up.
///
/// ```compile_fail,E0603
/// let name = agent_core::ids::InvestigatorName(String::from("Not Valid"));
/// ```
///
/// ```compile_fail,E0308
/// # fn main() -> Result<(), agent_core::ids::InvalidName> {
/// let name: agent_core::ids::AgentName = "inv-opus".parse::<agent_core::ids::InvestigatorName>()?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InvestigatorName(String);

text_newtype!(InvestigatorName, InvalidName, |text: &str| check_name(
    NameKind::Investigator,
    text
));

/// Which kind of name a text was parsed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameKind {
    /// An [`AgentName`].
    Agent,
    /// An [`InvestigatorName`].
    Investigator,
}

impl fmt::Display for NameKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Agent => "agent name",
            Self::Investigator => "investigator name",
        })
    }
}

/// The rule of [`AgentName`] and [`InvestigatorName`] that a text breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameRule {
    /// The text is empty.
    Empty,
    /// The text is longer than [`MAX_NAME_LEN`] bytes.
    TooLong,
    /// The first character is not a lowercase ASCII letter.
    FirstNotLetter,
    /// This character is not `a-z`, `0-9` or `-`.
    ForbiddenChar(char),
    /// The last character is `-`.
    LastHyphen,
}

impl fmt::Display for NameRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("it is empty"),
            Self::TooLong => write!(f, "it is longer than {MAX_NAME_LEN} bytes"),
            Self::FirstNotLetter => f.write_str("it must start with a letter a-z"),
            Self::ForbiddenChar(c) => write!(f, "{c:?} is not allowed: use a-z, 0-9 and '-'"),
            Self::LastHyphen => f.write_str("it must not end with '-'"),
        }
    }
}

/// Why a text is not an [`AgentName`] or an [`InvestigatorName`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} {value:?}: {rule}")]
pub struct InvalidName {
    /// The kind of name the text was parsed as.
    pub kind: NameKind,
    /// The refused text.
    pub value: String,
    /// The rule the text breaks.
    pub rule: NameRule,
}

/// Checks `text` against the rules of a name of `kind`.
fn check_name(kind: NameKind, text: &str) -> Result<(), InvalidName> {
    let refuse = |rule| {
        Err(InvalidName {
            kind,
            value: text.to_owned(),
            rule,
        })
    };
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return refuse(NameRule::Empty);
    };
    if text.len() > MAX_NAME_LEN {
        return refuse(NameRule::TooLong);
    }
    if !first.is_ascii_lowercase() {
        return refuse(NameRule::FirstNotLetter);
    }
    if let Some(bad) = chars.find(|&c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')) {
        return refuse(NameRule::ForbiddenChar(bad));
    }
    if text.ends_with('-') {
        return refuse(NameRule::LastHyphen);
    }
    Ok(())
}

/// The source of the current time for generated IDs: the daemon passes the system clock, tests a fixed time.
pub trait Clock {
    /// The current time, in UTC.
    fn now(&self) -> UtcDateTime;
}

/// ID of a team, such as `team-20261111-153601-k3x9`: the prefix, the UTC date and time it was made, four random
/// characters.
///
/// Refused, with the [`InvalidId`] variant: a text without the prefix `team-` (an ID of another kind among them),
/// one that is not `team-YYYYMMDD-HHMMSS-xxxx` with `x` in `0-9` and `a-z` (uppercase, non-ASCII digits, a missing
/// or extra part), and digits that name no real date or time of day (month 13, February 30, hour 24, second 60).
/// IDs of one kind sort by the time they were made.
///
/// ```
/// # fn main() -> Result<(), agent_core::ids::InvalidId> {
/// let id: agent_core::ids::TeamId = "team-20261111-153601-k3x9".parse()?;
/// assert_eq!(id.to_string(), "team-20261111-153601-k3x9");
/// # Ok(())
/// # }
/// ```
///
/// ```compile_fail,E0603
/// let id = agent_core::ids::TeamId(String::from("x"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TeamId(String);

generated_id!(TeamId, IdKind::Team);

/// ID of a round — the work of a team on one item of its task list — such as `round-20261111-153601-k3x9`: the rules
/// of [`TeamId`] with the prefix `round-`.
///
/// ```compile_fail,E0603
/// let id = agent_core::ids::RoundId(String::from("x"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RoundId(String);

generated_id!(RoundId, IdKind::Round);

impl RoundId {
    /// The ID of the round at `position` (counted from 0) of the team `team`, for a state that was written before
    /// rounds had IDs: the same team and position always give the same ID.
    #[must_use]
    pub fn derive(_team: &TeamId, _position: u32) -> Self {
        Self(String::new())
    }
}

/// ID of a task — one agent's work, solo or in a team — such as `task-20261111-153601-k3x9`: the rules of
/// [`TeamId`] with the prefix `task-`.
///
/// ```compile_fail,E0603
/// let id = agent_core::ids::TaskId(String::from("x"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskId(String);

generated_id!(TaskId, IdKind::Task);

/// ID of one run of a model CLI for a task, such as `run-20261111-153601-k3x9`: the rules of [`TeamId`] with the
/// prefix `run-`.
///
/// ```compile_fail,E0603
/// let id = agent_core::ids::RunId(String::from("x"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RunId(String);

generated_id!(RunId, IdKind::Run);

/// Which kind of generated ID a text was parsed as, or an ID was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdKind {
    /// A [`TeamId`].
    Team,
    /// A [`TaskId`].
    Task,
    /// A [`RunId`].
    Run,
    /// A [`RoundId`].
    Round,
}

impl IdKind {
    /// The text before the first `-` of an ID of this kind.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Task => "task",
            Self::Run => "run",
            Self::Round => "",
        }
    }
}

impl fmt::Display for IdKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ID", self.prefix())
    }
}

/// Why a text is not a [`TeamId`], [`RoundId`], [`TaskId`] or [`RunId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidId {
    /// The text does not start with the kind's prefix and `-`; it may be an ID of another kind.
    #[error("invalid {kind} {value:?}: it must start with \"{}-\"", .kind.prefix())]
    Prefix {
        /// The kind the text was parsed as.
        kind: IdKind,
        /// The refused text.
        value: String,
    },
    /// After the prefix, the text is not `YYYYMMDD-HHMMSS-xxxx` with `x` in `0-9` and `a-z`.
    #[error("invalid {kind} {value:?}: expected {}-YYYYMMDD-HHMMSS-xxxx with x in 0-9 and a-z", .kind.prefix())]
    Shape {
        /// The kind the text was parsed as.
        kind: IdKind,
        /// The refused text.
        value: String,
    },
    /// The digits name no real date or time of day.
    #[error("invalid {kind} {value:?}: no such date and time")]
    Timestamp {
        /// The kind the text was parsed as.
        kind: IdKind,
        /// The refused text.
        value: String,
        /// The calendar's reason: which component is out of range.
        source: ComponentRange,
    },
}

/// Why no ID could be generated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot generate a {kind}: the clock says year {year}, outside 0000-9999")]
pub struct GenerateIdError {
    /// The kind of ID that was asked for.
    pub kind: IdKind,
    /// The clock's year.
    pub year: i32,
}

/// The fields of `YYYYMMDD-HHMMSS-xxxx`, read but not yet checked against the calendar.
#[derive(Debug)]
struct Stamp {
    /// Year, 0–9999.
    year: u16,
    /// Month, 0–99 as read.
    month: u8,
    /// Day of the month, 0–99 as read.
    day: u8,
    /// Hour, 0–99 as read.
    hour: u8,
    /// Minute, 0–99 as read.
    minute: u8,
    /// Second, 0–99 as read.
    second: u8,
}

impl Stamp {
    /// Reads `YYYYMMDD-HHMMSS-xxxx`; `None` when `text` has another shape.
    fn read(text: &str) -> Option<Self> {
        let mut bytes = text.bytes();
        let century = two_digits(&mut bytes)?;
        let stamp = Self {
            year: u16::from(century) * 100 + u16::from(two_digits(&mut bytes)?),
            month: two_digits(&mut bytes)?,
            day: two_digits(&mut bytes)?,
            hour: separator(&mut bytes).and_then(|()| two_digits(&mut bytes))?,
            minute: two_digits(&mut bytes)?,
            second: two_digits(&mut bytes)?,
        };
        separator(&mut bytes)?;
        for _ in 0..SUFFIX_LEN {
            bytes.next().filter(|b| b.is_ascii_digit() || b.is_ascii_lowercase())?;
        }
        bytes.next().is_none().then_some(stamp)
    }

    /// Checks that the fields name a real date and time of day.
    fn check(&self) -> Result<(), ComponentRange> {
        let month = Month::try_from(self.month)?;
        Date::from_calendar_date(i32::from(self.year), month, self.day)?;
        Time::from_hms(self.hour, self.minute, self.second)?;
        Ok(())
    }
}

/// Takes the next byte of `bytes` as a decimal digit.
fn digit(bytes: &mut impl Iterator<Item = u8>) -> Option<u8> {
    bytes.next().filter(u8::is_ascii_digit).map(|b| b - b'0')
}

/// Takes the next two bytes of `bytes` as a two-digit decimal number.
fn two_digits(bytes: &mut impl Iterator<Item = u8>) -> Option<u8> {
    Some(digit(bytes)? * 10 + digit(bytes)?)
}

/// Takes the next byte of `bytes` when it is `-`.
fn separator(bytes: &mut impl Iterator<Item = u8>) -> Option<()> {
    bytes.next().filter(|&b| b == b'-').map(|_| ())
}

/// Checks `text` against the rules of a generated ID of `kind`.
fn check_id(kind: IdKind, text: &str) -> Result<(), InvalidId> {
    let Some(rest) = text.strip_prefix(kind.prefix()).and_then(|rest| rest.strip_prefix('-')) else {
        return Err(InvalidId::Prefix {
            kind,
            value: text.to_owned(),
        });
    };
    let Some(stamp) = Stamp::read(rest) else {
        return Err(InvalidId::Shape {
            kind,
            value: text.to_owned(),
        });
    };
    stamp.check().map_err(|source| InvalidId::Timestamp {
        kind,
        value: text.to_owned(),
        source,
    })
}

/// The text of a new ID of `kind`: the time `clock` gives and [`SUFFIX_LEN`] characters drawn from `rng`.
fn generate_id(kind: IdKind, clock: &impl Clock, rng: &mut impl Rng) -> Result<String, GenerateIdError> {
    let now = clock.now();
    let year = now.year();
    if !(0..=MAX_YEAR).contains(&year) {
        return Err(GenerateIdError { kind, year });
    }
    let suffix: String = (0..SUFFIX_LEN)
        .map(|_| suffix_char(rng.random_range(0..SUFFIX_RADIX)))
        .collect();
    Ok(format!(
        "{}-{year:04}{:02}{:02}-{:02}{:02}{:02}-{suffix}",
        kind.prefix(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    ))
}

/// The suffix character for `n`, which is below [`SUFFIX_RADIX`]: `0-9`, then `a-z`.
///
/// # Panics
/// In debug builds, when `n` is not below [`SUFFIX_RADIX`].
fn suffix_char(n: u8) -> char {
    debug_assert!(n < SUFFIX_RADIX, "suffix digit {n} out of range");
    char::from(if n < 10 { b'0' + n } else { b'a' + (n - 10) })
}

/// ID of an item of a task list, such as `2.3`: item 3 of feature 2.
///
/// Both numbers are decimal, start at 1, have no leading zeros and are at most 65535. Refused, with the
/// [`InvalidItemId`] variant: anything but two numbers joined by one `.` (`2`, `2.`, `.3`, `2.3.1`, `+2.3`, ` 2.3`,
/// non-ASCII digits), a leading zero (`02.3`), a zero (`0.1`, `2.0`) and a number above 65535. IDs sort by feature,
/// then item: `1.9` < `1.10` < `2.1`.
///
/// ```
/// # fn main() -> Result<(), agent_core::ids::InvalidItemId> {
/// let id: agent_core::ids::ItemId = "2.3".parse()?;
/// assert_eq!((id.feature().get(), id.item().get()), (2, 3));
/// # Ok(())
/// # }
/// ```
///
/// ```compile_fail,E0451
/// use std::num::NonZeroU16;
/// let one = NonZeroU16::MIN;
/// let id = agent_core::ids::ItemId { feature: one, item: one };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ItemId {
    /// Number of the feature the item belongs to.
    feature: NonZeroU16,
    /// Number of the item within its feature.
    item: NonZeroU16,
}

impl ItemId {
    /// Number of the feature the item belongs to: `2` in `2.3`.
    #[must_use]
    pub const fn feature(self) -> NonZeroU16 {
        self.feature
    }

    /// Number of the item within its feature: `3` in `2.3`.
    #[must_use]
    pub const fn item(self) -> NonZeroU16 {
        self.item
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.feature, self.item)
    }
}

impl FromStr for ItemId {
    type Err = InvalidItemId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let Some((feature, item)) = text.split_once('.') else {
            return Err(InvalidItemId::Shape { value: text.to_owned() });
        };
        Ok(Self {
            feature: item_number(text, ItemIdPart::Feature, feature)?,
            item: item_number(text, ItemIdPart::Item, item)?,
        })
    }
}

impl TryFrom<String> for ItemId {
    type Error = InvalidItemId;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<ItemId> for String {
    fn from(id: ItemId) -> Self {
        id.to_string()
    }
}

/// The `part` number of the item ID `text`, whose text is `digits`.
fn item_number(text: &str, part: ItemIdPart, digits: &str) -> Result<NonZeroU16, InvalidItemId> {
    let value = || text.to_owned();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(InvalidItemId::Shape { value: value() });
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return Err(InvalidItemId::LeadingZero { value: value(), part });
    }
    let number = digits.parse::<u16>().map_err(|source| InvalidItemId::TooLarge {
        value: value(),
        part,
        source,
    })?;
    NonZeroU16::new(number).ok_or_else(|| InvalidItemId::Zero { value: value(), part })
}

/// Which number of an item ID breaks a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemIdPart {
    /// The feature number, before the `.`.
    Feature,
    /// The item number, after the `.`.
    Item,
}

impl fmt::Display for ItemIdPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Feature => "feature",
            Self::Item => "item",
        })
    }
}

/// Why a text is not an [`ItemId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidItemId {
    /// The text is not two decimal numbers joined by one `.`.
    #[error("invalid item ID {value:?}: expected FEATURE.ITEM, such as 2.3")]
    Shape {
        /// The refused text.
        value: String,
    },
    /// A number has a leading zero.
    #[error("invalid item ID {value:?}: the {part} number has a leading zero")]
    LeadingZero {
        /// The refused text.
        value: String,
        /// The number with the leading zero.
        part: ItemIdPart,
    },
    /// A number is 0; numbering starts at 1.
    #[error("invalid item ID {value:?}: the {part} number is 0, numbers start at 1")]
    Zero {
        /// The refused text.
        value: String,
        /// The number that is 0.
        part: ItemIdPart,
    },
    /// A number is above 65535.
    #[error("invalid item ID {value:?}: the {part} number is above 65535")]
    TooLarge {
        /// The refused text.
        value: String,
        /// The number that is too large.
        part: ItemIdPart,
        /// The integer parser's reason.
        source: ParseIntError,
    },
}

/// A literal git branch name, such as `agents/codex-1/task-20261111-153601-k3x9`, checked without running git.
///
/// Uses Git's literal ref rules with a byte-length bound and an explicit refusal of `@` (revision shorthand).
/// Unlike `git check-ref-format --branch`, it never expands checkout-history expressions.
///
/// Refused, with the [`BranchRule`] it breaks: the empty text, one longer than [`MAX_BRANCH_LEN`] bytes, `HEAD`,
/// `@`, a first `-`, ASCII control characters, space, `~`, `^`, `:`, `?`, `*`, `[` and `\`, `..`, `@{`, a last `.`,
/// an empty component (a first or last `/`, `//`), a component starting with `.` and a component ending with
/// `.lock`. Other non-ASCII text is accepted, as git accepts it.
///
/// ```
/// # fn main() -> Result<(), agent_core::ids::InvalidBranchName> {
/// let branch: agent_core::ids::BranchName = "agents/codex-1/task-20261111-153601-k3x9".parse()?;
/// assert_eq!(branch.as_str(), "agents/codex-1/task-20261111-153601-k3x9");
/// # Ok(())
/// # }
/// ```
///
/// ```compile_fail,E0603
/// let branch = agent_core::ids::BranchName(String::from("a..b"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BranchName(String);

text_newtype!(BranchName, InvalidBranchName, check_branch);

/// The rule of [`BranchName`] that a text breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchRule {
    /// The text is empty.
    Empty,
    /// The text is longer than [`MAX_BRANCH_LEN`] bytes.
    TooLong,
    /// The text is `HEAD`, which git reserves.
    Head,
    /// The text is `@`, which git reads as `HEAD`.
    At,
    /// The first character is `-`: git would read the name as an option.
    FirstHyphen,
    /// This character is an ASCII control character or one of space, `~`, `^`, `:`, `?`, `*`, `[` and `\`.
    ForbiddenChar(char),
    /// The text contains `..`.
    DoubleDot,
    /// The text contains `@{`.
    AtBrace,
    /// The last character is `.`.
    LastDot,
    /// A component between `/` is empty: a first or last `/`, or `//`.
    EmptyComponent,
    /// A component starts with `.`.
    ComponentStartsWithDot,
    /// A component ends with `.lock`.
    ComponentEndsWithLock,
}

impl fmt::Display for BranchRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("it is empty"),
            Self::TooLong => write!(f, "it is longer than {MAX_BRANCH_LEN} bytes"),
            Self::Head => f.write_str("HEAD is reserved"),
            Self::At => f.write_str("'@' is reserved"),
            Self::FirstHyphen => f.write_str("it must not start with '-'"),
            Self::ForbiddenChar(c) => write!(f, "{c:?} is not allowed"),
            Self::DoubleDot => f.write_str("it must not contain '..'"),
            Self::AtBrace => f.write_str("it must not contain '@{'"),
            Self::LastDot => f.write_str("it must not end with '.'"),
            Self::EmptyComponent => f.write_str("it must not start or end with '/' or contain '//'"),
            Self::ComponentStartsWithDot => f.write_str("no part between '/' may start with '.'"),
            Self::ComponentEndsWithLock => f.write_str("no part between '/' may end with '.lock'"),
        }
    }
}

/// Why a text is not a [`BranchName`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid branch name {value:?}: {rule}")]
pub struct InvalidBranchName {
    /// The refused text.
    pub value: String,
    /// The rule the text breaks.
    pub rule: BranchRule,
}

/// Checks `text` against the rules of a branch name.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "`.lock` is git's reserved component suffix, not a file extension, and git compares it case-sensitively"
)]
fn check_branch(text: &str) -> Result<(), InvalidBranchName> {
    let refuse = |rule| {
        Err(InvalidBranchName {
            value: text.to_owned(),
            rule,
        })
    };
    if text.is_empty() {
        return refuse(BranchRule::Empty);
    }
    if text.len() > MAX_BRANCH_LEN {
        return refuse(BranchRule::TooLong);
    }
    if text == "HEAD" {
        return refuse(BranchRule::Head);
    }
    if text == "@" {
        return refuse(BranchRule::At);
    }
    if text.starts_with('-') {
        return refuse(BranchRule::FirstHyphen);
    }
    if let Some(bad) = text
        .chars()
        .find(|c| c.is_ascii_control() || BRANCH_FORBIDDEN.contains(c))
    {
        return refuse(BranchRule::ForbiddenChar(bad));
    }
    if text.contains("..") {
        return refuse(BranchRule::DoubleDot);
    }
    if text.contains("@{") {
        return refuse(BranchRule::AtBrace);
    }
    if text.ends_with('.') {
        return refuse(BranchRule::LastDot);
    }
    for component in text.split('/') {
        if component.is_empty() {
            return refuse(BranchRule::EmptyComponent);
        }
        if component.starts_with('.') {
            return refuse(BranchRule::ComponentStartsWithDot);
        }
        if component.ends_with(".lock") {
            return refuse(BranchRule::ComponentEndsWithLock);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rand::SeedableRng;
    use rand::rngs::Xoshiro256PlusPlus;
    use serde_json::error::Category;

    use super::*;

    /// A clock that always says the same time.
    struct FixedClock(UtcDateTime);

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            self.0
        }
    }

    /// The UTC time `year-month-day hour:minute:second`.
    fn at(year: i32, month: Month, day: u8, (hour, minute, second): (u8, u8, u8)) -> UtcDateTime {
        UtcDateTime::new(
            Date::from_calendar_date(year, month, day).unwrap(),
            Time::from_hms(hour, minute, second).unwrap(),
        )
    }

    /// The random source of a test: seeded, so every run draws the same values.
    fn rng(seed: u64) -> Xoshiro256PlusPlus {
        Xoshiro256PlusPlus::seed_from_u64(seed)
    }

    /// Asserts that `T` parses `text` to a value whose text, `String` and JSON forms give `text` back.
    fn assert_round_trip<T>(text: &str)
    where
        T: FromStr<Err: fmt::Debug> + TryFrom<String, Error: fmt::Debug> + fmt::Display + Serialize,
        T: for<'de> Deserialize<'de> + Into<String> + PartialEq + fmt::Debug + Clone,
    {
        let value: T = text.parse().unwrap();
        assert_eq!(value.to_string(), text);
        assert_eq!(T::try_from(text.to_owned()).unwrap(), value);
        assert_eq!(value.clone().into(), text);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, serde_json::to_string(text).unwrap());
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
    }

    /// Asserts that deserializing the JSON string `text` as `T` fails as invalid data.
    fn assert_serde_refuses<T: for<'de> Deserialize<'de> + fmt::Debug>(text: &str) {
        let json = serde_json::to_string(text).unwrap();
        let err = serde_json::from_str::<T>(&json).unwrap_err();
        assert_eq!(err.classify(), Category::Data, "{text:?}");
    }

    const VALID_NAMES: &[&str] = &["a", "z9", "codex-1", "inv-opus", "openrouter-haiku", "a--b", "a1-b2-c3"];

    #[test]
    fn agent_and_investigator_names_round_trip() {
        let longest = "a".repeat(MAX_NAME_LEN);
        for text in VALID_NAMES.iter().copied().chain([longest.as_str()]) {
            assert_round_trip::<AgentName>(text);
            assert_round_trip::<InvestigatorName>(text);
        }
    }

    #[test]
    fn random_valid_names_round_trip() {
        const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789-";
        let mut rng = rng(1);
        for _ in 0..1000 {
            let len = rng.random_range(1..=MAX_NAME_LEN);
            let mut name = String::from(char::from(rng.random_range(b'a'..=b'z')));
            name.extend((1..len).map(|_| char::from(REST[rng.random_range(0..REST.len())])));
            if name.ends_with('-') {
                name.pop();
                name.push('x');
            }
            assert_round_trip::<AgentName>(&name);
            assert_round_trip::<InvestigatorName>(&name);
        }
    }

    #[test]
    fn names_refuse_every_invalid_value_with_its_rule() {
        let too_long_ascii = "a".repeat(MAX_NAME_LEN + 1);
        let too_long_utf8 = "ż".repeat(MAX_NAME_LEN / 2 + 1);
        let cases = [
            ("", NameRule::Empty),
            (too_long_ascii.as_str(), NameRule::TooLong),
            (too_long_utf8.as_str(), NameRule::TooLong),
            ("1abc", NameRule::FirstNotLetter),
            ("-abc", NameRule::FirstNotLetter),
            ("Abc", NameRule::FirstNotLetter),
            ("żółw", NameRule::FirstNotLetter),
            ("aBc", NameRule::ForbiddenChar('B')),
            ("a_b", NameRule::ForbiddenChar('_')),
            ("a.b", NameRule::ForbiddenChar('.')),
            ("a/b", NameRule::ForbiddenChar('/')),
            ("a b", NameRule::ForbiddenChar(' ')),
            ("ab\n", NameRule::ForbiddenChar('\n')),
            ("aż", NameRule::ForbiddenChar('ż')),
            ("abc-", NameRule::LastHyphen),
        ];
        for (text, rule) in cases {
            let expected = |kind| InvalidName {
                kind,
                value: text.to_owned(),
                rule,
            };
            assert_eq!(text.parse::<AgentName>(), Err(expected(NameKind::Agent)), "{text:?}");
            assert_eq!(
                text.parse::<InvestigatorName>(),
                Err(expected(NameKind::Investigator)),
                "{text:?}"
            );
            assert_eq!(
                AgentName::try_from(text.to_owned()),
                Err(expected(NameKind::Agent)),
                "{text:?}"
            );
            assert_serde_refuses::<AgentName>(text);
            assert_serde_refuses::<InvestigatorName>(text);
        }
    }

    /// Parses `text` as the generated ID of `kind` and returns its text.
    fn parse_id(kind: IdKind, text: &str) -> Result<String, InvalidId> {
        match kind {
            IdKind::Team => text.parse::<TeamId>().map(String::from),
            IdKind::Task => text.parse::<TaskId>().map(String::from),
            IdKind::Run => text.parse::<RunId>().map(String::from),
            IdKind::Round => text.parse::<RoundId>().map(String::from),
        }
    }

    /// Generates an ID of `kind` and returns its text.
    fn generate(kind: IdKind, clock: &impl Clock, rng: &mut impl Rng) -> Result<String, GenerateIdError> {
        match kind {
            IdKind::Team => TeamId::generate(clock, rng).map(String::from),
            IdKind::Task => TaskId::generate(clock, rng).map(String::from),
            IdKind::Run => RunId::generate(clock, rng).map(String::from),
            IdKind::Round => RoundId::generate(clock, rng).map(String::from),
        }
    }

    const KINDS: [IdKind; 3] = [IdKind::Team, IdKind::Task, IdKind::Run];

    #[test]
    fn generated_ids_round_trip() {
        for stamp in [
            "20261111-153601-k3x9",
            "20240229-000000-0000",
            "00000101-000000-zzzz",
            "99991231-235959-a1b2",
        ] {
            assert_round_trip::<TeamId>(&format!("team-{stamp}"));
            assert_round_trip::<TaskId>(&format!("task-{stamp}"));
            assert_round_trip::<RunId>(&format!("run-{stamp}"));
        }
    }

    #[test]
    fn a_generated_id_carries_its_kind_the_clock_time_and_a_random_suffix() {
        let clock = FixedClock(at(2026, Month::November, 11, (15, 36, 1)));
        for kind in KINDS {
            let id = generate(kind, &clock, &mut rng(2)).unwrap();

            let suffix = id.strip_prefix(&format!("{}-20261111-153601-", kind.prefix())).unwrap();
            assert_eq!(suffix.len(), SUFFIX_LEN, "{id}");
            assert!(
                suffix.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase()),
                "{id}"
            );
            assert_eq!(parse_id(kind, &id), Ok(id.clone()));
        }
    }

    #[test]
    fn the_same_seed_generates_the_same_id_and_another_seed_another() {
        let clock = FixedClock(at(2026, Month::November, 11, (15, 36, 1)));

        let first = TeamId::generate(&clock, &mut rng(3)).unwrap();

        assert_eq!(TeamId::generate(&clock, &mut rng(3)).unwrap(), first);
        assert_ne!(TeamId::generate(&clock, &mut rng(4)).unwrap(), first);
    }

    #[test]
    fn generated_suffixes_use_every_digit_and_lowercase_letter() {
        let clock = FixedClock(at(2026, Month::November, 11, (15, 36, 1)));
        let mut rng = rng(5);
        let mut seen = BTreeSet::new();
        for _ in 0..1000 {
            let id = RunId::generate(&clock, &mut rng).unwrap();
            seen.extend(id.as_str().chars().rev().take(SUFFIX_LEN));
        }

        let expected: BTreeSet<char> = ('0'..='9').chain('a'..='z').collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn ids_generated_at_random_times_parse_back_to_equal_values() {
        // 0000-01-01T00:00:00Z and 9999-12-31T23:59:59Z, the range a four-digit year can name.
        const FIRST: i64 = -62_167_219_200;
        const LAST: i64 = 253_402_300_799;
        let mut rng = rng(6);
        for _ in 0..1000 {
            let clock = FixedClock(UtcDateTime::from_unix_timestamp(rng.random_range(FIRST..=LAST)).unwrap());
            for kind in KINDS {
                let id = generate(kind, &clock, &mut rng).unwrap();
                assert_eq!(parse_id(kind, &id), Ok(id.clone()));
            }
        }
    }

    #[test]
    fn ids_are_generated_at_the_first_and_last_second_of_four_digit_years() {
        let first = FixedClock(at(0, Month::January, 1, (0, 0, 0)));
        let last = FixedClock(at(9999, Month::December, 31, (23, 59, 59)));

        let early = TaskId::generate(&first, &mut rng(7)).unwrap();
        let late = TaskId::generate(&last, &mut rng(7)).unwrap();

        assert!(early.as_str().starts_with("task-00000101-000000-"), "{early}");
        assert!(late.as_str().starts_with("task-99991231-235959-"), "{late}");
    }

    #[test]
    fn no_id_is_generated_before_year_0() {
        let clock = FixedClock(at(-1, Month::December, 31, (23, 59, 59)));
        for kind in KINDS {
            assert_eq!(
                generate(kind, &clock, &mut rng(8)),
                Err(GenerateIdError { kind, year: -1 })
            );
        }
    }

    #[test]
    fn ids_of_one_kind_sort_by_the_time_they_were_made() {
        let earlier: TeamId = "team-20261111-153601-zzzz".parse().unwrap();
        let later: TeamId = "team-20261111-153602-0000".parse().unwrap();

        assert!(earlier < later);
    }

    /// What a test expects when it parses an invalid ID.
    #[derive(Debug, Clone, Copy)]
    enum Refusal {
        /// [`InvalidId::Prefix`].
        Prefix,
        /// [`InvalidId::Shape`].
        Shape,
        /// [`InvalidId::Timestamp`], with the name of the component that is out of range.
        Timestamp(&'static str),
    }

    #[test]
    fn ids_refuse_every_invalid_value_with_its_variant() {
        // `{p}` stands for the prefix of the kind being parsed, `{o}` for that of another kind.
        let cases = [
            ("", Refusal::Prefix),
            ("20261111-153601-k3x9", Refusal::Prefix),
            ("{o}-20261111-153601-k3x9", Refusal::Prefix),
            ("{P}-20261111-153601-k3x9", Refusal::Prefix),
            ("{p}20261111-153601-k3x9", Refusal::Prefix),
            ("{p}s-20261111-153601-k3x9", Refusal::Prefix),
            (" {p}-20261111-153601-k3x9", Refusal::Prefix),
            ("{p}-", Refusal::Shape),
            ("{p}-2026111-153601-k3x9", Refusal::Shape),
            ("{p}-20261111-15360-k3x9", Refusal::Shape),
            ("{p}-20261111-1536011-k3x9", Refusal::Shape),
            ("{p}-20261111-153601-k3x", Refusal::Shape),
            ("{p}-20261111-153601-k3x9a", Refusal::Shape),
            ("{p}-20261111-153601-k3x9-", Refusal::Shape),
            ("{p}-20261111-153601-K3X9", Refusal::Shape),
            ("{p}-20261111-153601-k3ż9", Refusal::Shape),
            ("{p}-20261111_153601-k3x9", Refusal::Shape),
            ("{p}-2026111١-153601-k3x9", Refusal::Shape),
            ("{p}-20261111-153601-k3x9 ", Refusal::Shape),
            ("{p}-20261311-153601-k3x9", Refusal::Timestamp("month")),
            ("{p}-20260011-153601-k3x9", Refusal::Timestamp("month")),
            ("{p}-20261100-153601-k3x9", Refusal::Timestamp("day")),
            ("{p}-20260230-153601-k3x9", Refusal::Timestamp("day")),
            ("{p}-20250229-153601-k3x9", Refusal::Timestamp("day")),
            ("{p}-20261111-240000-k3x9", Refusal::Timestamp("hour")),
            ("{p}-20261111-236000-k3x9", Refusal::Timestamp("minute")),
            ("{p}-20261111-235960-k3x9", Refusal::Timestamp("second")),
        ];
        for kind in KINDS {
            let other = if kind == IdKind::Team {
                IdKind::Task
            } else {
                IdKind::Team
            };
            for (pattern, refusal) in cases {
                let text = pattern
                    .replace("{p}", kind.prefix())
                    .replace("{P}", &kind.prefix().to_uppercase())
                    .replace("{o}", other.prefix());
                let value = text.clone();
                match (parse_id(kind, &text), refusal) {
                    (Err(err), Refusal::Prefix) => assert_eq!(err, InvalidId::Prefix { kind, value }),
                    (Err(err), Refusal::Shape) => assert_eq!(err, InvalidId::Shape { kind, value }),
                    (
                        Err(InvalidId::Timestamp {
                            kind: k,
                            value: v,
                            source,
                        }),
                        Refusal::Timestamp(component),
                    ) => {
                        assert_eq!((k, v, source.name()), (kind, value, component));
                    }
                    (result, refusal) => panic!("{text:?}: expected {refusal:?}, got {result:?}"),
                }
                match kind {
                    IdKind::Team => assert_serde_refuses::<TeamId>(&text),
                    IdKind::Task => assert_serde_refuses::<TaskId>(&text),
                    IdKind::Run => assert_serde_refuses::<RunId>(&text),
                    IdKind::Round => assert_serde_refuses::<RoundId>(&text),
                }
            }
        }
    }

    #[test]
    fn round_ids_round_trip() {
        for stamp in [
            "20261111-153601-k3x9",
            "20240229-000000-0000",
            "00000101-000000-zzzz",
            "99991231-235959-a1b2",
        ] {
            assert_round_trip::<RoundId>(&format!("round-{stamp}"));
        }
    }

    #[test]
    fn a_round_id_is_generated_with_the_prefix_round_the_clock_time_and_a_random_suffix() {
        let clock = FixedClock(at(2026, Month::November, 11, (15, 36, 1)));

        let id = RoundId::generate(&clock, &mut rng(20)).unwrap();

        let suffix = id.as_str().strip_prefix("round-20261111-153601-").unwrap();
        assert_eq!(suffix.len(), SUFFIX_LEN, "{id}");
        assert_eq!(id.as_str().parse::<RoundId>(), Ok(id.clone()));
        assert_eq!(IdKind::Round.prefix(), "round");
        assert_eq!(IdKind::Round.to_string(), "round ID");
    }

    #[test]
    fn a_round_id_refuses_the_ids_of_other_kinds_and_every_malformed_form() {
        let kind = IdKind::Round;
        let prefix = |text: &str| {
            Err(InvalidId::Prefix {
                kind,
                value: text.to_owned(),
            })
        };
        let shape = |text: &str| {
            Err(InvalidId::Shape {
                kind,
                value: text.to_owned(),
            })
        };
        for text in [
            "",
            "20261111-153601-k3x9",
            "team-20261111-153601-k3x9",
            "task-20261111-153601-k3x9",
            "run-20261111-153601-k3x9",
            "Round-20261111-153601-k3x9",
            "rounds-20261111-153601-k3x9",
            "round20261111-153601-k3x9",
            " round-20261111-153601-k3x9",
        ] {
            assert_eq!(parse_id(kind, text), prefix(text), "{text:?}");
            assert_serde_refuses::<RoundId>(text);
        }
        for text in [
            "round-",
            "round-20261111-153601",
            "round-2026111-153601-k3x9",
            "round-20261111-153601-K3X9",
            "round-20261111-153601-k3x9a",
            "round-20261111-153601-k3x9 ",
            "round-20261111_153601-k3x9",
        ] {
            assert_eq!(parse_id(kind, text), shape(text), "{text:?}");
            assert_serde_refuses::<RoundId>(text);
        }
        for (text, component) in [
            ("round-20261311-153601-k3x9", "month"),
            ("round-20260230-153601-k3x9", "day"),
            ("round-20261111-240000-k3x9", "hour"),
            ("round-20261111-236000-k3x9", "minute"),
            ("round-20261111-235960-k3x9", "second"),
        ] {
            let Err(InvalidId::Timestamp {
                kind: found,
                value,
                source,
            }) = parse_id(kind, text)
            else {
                panic!("{text:?} was not refused for its time");
            };
            assert_eq!((found, value.as_str(), source.name()), (kind, text, component));
            assert_serde_refuses::<RoundId>(text);
        }
    }

    #[test]
    fn a_round_id_made_by_derive_is_the_time_of_the_team_and_a_suffix_that_follows_the_position() {
        let team: TeamId = "team-20261111-153601-k3x9".parse().unwrap();

        let ids: Vec<String> = (0..3).map(|position| RoundId::derive(&team, position).into()).collect();

        assert_eq!(
            ids,
            [
                "round-20261111-153601-k3xa",
                "round-20261111-153601-k3xb",
                "round-20261111-153601-k3xc",
            ]
        );
    }

    #[test]
    fn the_suffix_of_a_derived_round_id_carries_over_and_wraps_around() {
        let carry: TeamId = "team-20261111-153601-00zz".parse().unwrap();
        let wrap: TeamId = "team-20261111-153601-zzzz".parse().unwrap();

        assert_eq!(RoundId::derive(&carry, 0).as_str(), "round-20261111-153601-0100");
        assert_eq!(RoundId::derive(&wrap, 0).as_str(), "round-20261111-153601-0000");
        assert_eq!(RoundId::derive(&wrap, 1).as_str(), "round-20261111-153601-0001");
    }

    #[test]
    fn deriving_the_id_of_the_same_round_twice_gives_the_same_id_and_other_teams_and_positions_other_ids() {
        let team: TeamId = "team-20261111-153601-k3x9".parse().unwrap();
        let other: TeamId = "team-20261111-153601-k3xa".parse().unwrap();

        assert_eq!(RoundId::derive(&team, 4), RoundId::derive(&team, 4));
        assert_ne!(RoundId::derive(&team, 4), RoundId::derive(&team, 5));
        assert_ne!(RoundId::derive(&team, 4), RoundId::derive(&other, 4));
    }

    #[test]
    fn derived_round_ids_of_random_teams_parse_and_differ_by_position() {
        for seed in 0..200 {
            let mut rng = rng(seed);
            let clock = FixedClock(UtcDateTime::from_unix_timestamp(rng.random_range(0..=253_402_300_799)).unwrap());
            let team = TeamId::generate(&clock, &mut rng).unwrap();
            let ids: Vec<RoundId> = (0..50).map(|position| RoundId::derive(&team, position)).collect();

            for id in &ids {
                assert_eq!(id.as_str().parse::<RoundId>().as_ref(), Ok(id), "seed {seed}: {id}");
                let stamp = |text: &str| text.split('-').skip(1).take(2).collect::<Vec<_>>().join("-");
                assert_eq!(stamp(id.as_str()), stamp(team.as_str()), "seed {seed}");
            }
            let distinct: BTreeSet<&RoundId> = ids.iter().collect();
            assert_eq!(distinct.len(), ids.len(), "seed {seed}: {team}");
        }
    }

    #[test]
    fn item_ids_round_trip_and_expose_their_numbers() {
        for (text, feature, item) in [
            ("1.1", 1, 1),
            ("2.3", 2, 3),
            ("10.12", 10, 12),
            ("65535.65535", 65535, 65535),
        ] {
            assert_round_trip::<ItemId>(text);
            let id: ItemId = text.parse().unwrap();
            assert_eq!((id.feature().get(), id.item().get()), (feature, item));
        }
    }

    #[test]
    fn random_item_ids_round_trip() {
        let mut rng = rng(9);
        for _ in 0..1000 {
            let text = format!("{}.{}", rng.random_range(1..=u16::MAX), rng.random_range(1..=u16::MAX));
            assert_round_trip::<ItemId>(&text);
        }
    }

    #[test]
    fn item_ids_sort_by_feature_then_item() {
        let ids: Vec<ItemId> = ["1.9", "1.10", "2.1"].iter().map(|t| t.parse().unwrap()).collect();

        let mut sorted = ids.clone();
        sorted.sort();

        assert_eq!(sorted, ids);
    }

    #[test]
    fn item_ids_refuse_every_value_that_is_not_two_numbers() {
        for text in [
            "", "2", "2.", ".3", ".", "2.3.1", "+2.3", "2.+3", "-1.1", " 2.3", "2.3 ", "2,3", "a.b", "١.٢", "2.3a",
        ] {
            let expected = InvalidItemId::Shape { value: text.to_owned() };
            assert_eq!(text.parse::<ItemId>(), Err(expected), "{text:?}");
            assert_serde_refuses::<ItemId>(text);
        }
    }

    #[test]
    fn item_ids_refuse_leading_zeros_and_zeros() {
        let cases = [
            (
                "02.3",
                InvalidItemId::LeadingZero {
                    value: "02.3".into(),
                    part: ItemIdPart::Feature,
                },
            ),
            (
                "00.1",
                InvalidItemId::LeadingZero {
                    value: "00.1".into(),
                    part: ItemIdPart::Feature,
                },
            ),
            (
                "2.03",
                InvalidItemId::LeadingZero {
                    value: "2.03".into(),
                    part: ItemIdPart::Item,
                },
            ),
            (
                "0.1",
                InvalidItemId::Zero {
                    value: "0.1".into(),
                    part: ItemIdPart::Feature,
                },
            ),
            (
                "2.0",
                InvalidItemId::Zero {
                    value: "2.0".into(),
                    part: ItemIdPart::Item,
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(text.parse::<ItemId>(), Err(expected), "{text:?}");
            assert_serde_refuses::<ItemId>(text);
        }
    }

    #[test]
    fn item_ids_refuse_numbers_above_65535() {
        for (text, expected_part) in [
            ("65536.1", ItemIdPart::Feature),
            ("1.65536", ItemIdPart::Item),
            ("99999999999999999999.1", ItemIdPart::Feature),
        ] {
            let Err(InvalidItemId::TooLarge { value, part, .. }) = text.parse::<ItemId>() else {
                panic!("{text:?} is not refused as too large");
            };
            assert_eq!((value.as_str(), part), (text, expected_part));
            assert_serde_refuses::<ItemId>(text);
        }
    }

    #[test]
    fn branch_names_round_trip() {
        let longest = "a".repeat(MAX_BRANCH_LEN);
        for text in [
            "main",
            "agents/codex-1/task-20261111-153601-k3x9",
            "feature/zażółć",
            "a.b",
            "a-b",
            "a@b",
            "a{b}",
            "heads/HEAD",
            "v1.2/x.locked",
            "x.lock.d",
            longest.as_str(),
        ] {
            assert_round_trip::<BranchName>(text);
        }
    }

    #[test]
    fn branch_names_refuse_every_invalid_value_with_its_rule() {
        let too_long = "a".repeat(MAX_BRANCH_LEN + 1);
        let cases = [
            ("", BranchRule::Empty),
            (too_long.as_str(), BranchRule::TooLong),
            ("HEAD", BranchRule::Head),
            ("@", BranchRule::At),
            ("-x", BranchRule::FirstHyphen),
            ("a b", BranchRule::ForbiddenChar(' ')),
            ("a~b", BranchRule::ForbiddenChar('~')),
            ("a^b", BranchRule::ForbiddenChar('^')),
            ("a:b", BranchRule::ForbiddenChar(':')),
            ("a?b", BranchRule::ForbiddenChar('?')),
            ("a*b", BranchRule::ForbiddenChar('*')),
            ("a[b", BranchRule::ForbiddenChar('[')),
            ("a\\b", BranchRule::ForbiddenChar('\\')),
            ("a\tb", BranchRule::ForbiddenChar('\t')),
            ("a\nb", BranchRule::ForbiddenChar('\n')),
            ("a\u{0}b", BranchRule::ForbiddenChar('\u{0}')),
            ("a\u{7f}b", BranchRule::ForbiddenChar('\u{7f}')),
            ("a..b", BranchRule::DoubleDot),
            ("a@{b", BranchRule::AtBrace),
            ("a.", BranchRule::LastDot),
            ("/a", BranchRule::EmptyComponent),
            ("a/", BranchRule::EmptyComponent),
            ("a//b", BranchRule::EmptyComponent),
            (".a", BranchRule::ComponentStartsWithDot),
            ("a/.b", BranchRule::ComponentStartsWithDot),
            ("a.lock", BranchRule::ComponentEndsWithLock),
            ("a.lock/b", BranchRule::ComponentEndsWithLock),
        ];
        for (text, rule) in cases {
            let expected = InvalidBranchName {
                value: text.to_owned(),
                rule,
            };
            assert_eq!(text.parse::<BranchName>(), Err(expected), "{text:?}");
            assert_serde_refuses::<BranchName>(text);
        }
    }
}
