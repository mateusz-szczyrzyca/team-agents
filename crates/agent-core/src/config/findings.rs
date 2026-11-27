//! Bounded configuration for findings included in a run's message.
use serde::Deserialize;

/// A key in the `[findings]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum FindingsKey {
    /// Retention in days, including the boundary day.
    KeepDays,
    /// Maximum number of findings in a run.
    MaxShown,
    /// Maximum total characters in the selected texts.
    MaxChars,
}

/// A findings configuration value lies outside its permitted range.
#[derive(Debug, thiserror::Error)]
#[error("invalid [findings] {key} value {value}")]
pub struct InvalidFindings {
    /// The key whose bounds were violated.
    pub key: FindingsKey,
    /// The rejected number.
    pub value: i64,
    /// A numeric conversion failure, when the number cannot fit in u16.
    #[source]
    source: Option<std::num::TryFromIntError>,
}

/// Validated `[findings]` bounds; absent sections keep the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Findings {
    /// Retention days, 0 through 365; 0 disables selection.
    keep_days: u16,
    /// Maximum findings per message, 1 through 20.
    max_shown: u16,
    /// Maximum characters in their texts, 500 through 10000.
    max_chars: u16,
}
impl Default for Findings {
    fn default() -> Self {
        Self {
            keep_days: 30,
            max_shown: 8,
            max_chars: 3000,
        }
    }
}
impl Findings {
    /// The retention period in days; zero disables selection.
    #[must_use]
    pub const fn keep_days(self) -> u16 {
        self.keep_days
    }
    /// The maximum findings in a message.
    #[must_use]
    pub const fn max_shown(self) -> u16 {
        self.max_shown
    }
    /// The maximum total character count of their texts.
    #[must_use]
    pub const fn max_chars(self) -> u16 {
        self.max_chars
    }
}

/// Unchecked TOML numbers; validation precedes construction of [`Findings`].
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RawFindings {
    /// The unchecked retention days.
    keep_days: i64,
    /// The unchecked message count.
    max_shown: i64,
    /// The unchecked character budget.
    max_chars: i64,
}
impl Default for RawFindings {
    fn default() -> Self {
        let defaults = Findings::default();
        Self {
            keep_days: i64::from(defaults.keep_days),
            max_shown: i64::from(defaults.max_shown),
            max_chars: i64::from(defaults.max_chars),
        }
    }
}
impl RawFindings {
    /// Checks all bounds before making settings available to decisions.
    ///
    /// # Errors
    /// Returns [`InvalidFindings`] naming the first key outside its inclusive bounds.
    pub(super) fn validate(self) -> Result<Findings, InvalidFindings> {
        Ok(Findings {
            keep_days: bound(self.keep_days, FindingsKey::KeepDays, 0..=365)?,
            max_shown: bound(self.max_shown, FindingsKey::MaxShown, 1..=20)?,
            max_chars: bound(self.max_chars, FindingsKey::MaxChars, 500..=10000)?,
        })
    }
}

/// Converts once at the boundary, then enforces the key's narrower domain range.
fn bound(value: i64, key: FindingsKey, range: std::ops::RangeInclusive<u16>) -> Result<u16, InvalidFindings> {
    let number = u16::try_from(value).map_err(|source| InvalidFindings {
        key,
        value,
        source: Some(source),
    })?;
    if !range.contains(&number) {
        return Err(InvalidFindings {
            key,
            value,
            source: None,
        });
    }
    Ok(number)
}

#[cfg(test)]
mod tests;
