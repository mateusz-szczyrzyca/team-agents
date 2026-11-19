//! Deserialization of maps and sets keyed by finding numbers ([`NonZeroU32`]).
//!
//! Below internally tagged enums (`#[serde(tag = ...)]`), serde buffers variant contents into an
//! intermediate tree where JSON object keys are stored as strings. Standard integer deserializers
//! reject string map keys with `invalid type: string "...", expected a nonzero u32`.
//!
//! The functions here accept finding numbers either as integer numbers or as strings.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Deserializer};

/// What a finding number must be, as the error of a refused key says it.
const POSITIVE: &str = "a positive integer";

/// What a finding number written as text must be: the text of a number, without sign or leading zeros.
const CANONICAL: &str = "a positive integer written without sign or leading zeros";

/// A finding number read from a JSON key or element; never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FindingKey(NonZeroU32);

/// Visitor that reads finding numbers from text or integers. serde forwards the other integer widths to
/// `visit_u64` and `visit_i64`, so these two cover every number a deserializer can hand over.
struct FindingKeyVisitor;

impl serde::de::Visitor<'_> for FindingKeyVisitor {
    type Value = FindingKey;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a positive integer finding number")
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<FindingKey, E> {
        parse_finding_number(text)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<FindingKey, E> {
        match u32::try_from(v) {
            Ok(number) => NonZeroU32::new(number)
                .map(FindingKey)
                .ok_or_else(|| refuse(&v, POSITIVE, None)),
            Err(source) => Err(refuse(&v, POSITIVE, Some(&source))),
        }
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<FindingKey, E> {
        match u32::try_from(v) {
            Ok(number) => NonZeroU32::new(number)
                .map(FindingKey)
                .ok_or_else(|| refuse(&v, POSITIVE, None)),
            Err(source) => Err(refuse(&v, POSITIVE, Some(&source))),
        }
    }
}

/// The error of a key that is not a finding number: it names the key as it was read and says what it must be.
/// `cause` is the reason the conversion failed, when there is one.
fn refuse<E: serde::de::Error>(key: &dyn fmt::Debug, expected: &str, cause: Option<&dyn fmt::Display>) -> E {
    match cause {
        Some(cause) => E::custom(format!("invalid finding number {key:?}: expected {expected}: {cause}")),
        None => E::custom(format!("invalid finding number {key:?}: expected {expected}")),
    }
}

/// Parses a finding number string into a [`FindingKey`].
///
/// Only the text that `serde_json` writes for a number is accepted: digits, the first of them not zero. `"+1"` and
/// `"01"` are refused, as they are when the same map is read from the state on its own.
fn parse_finding_number<E: serde::de::Error>(text: &str) -> Result<FindingKey, E> {
    let canonical = text.starts_with(|c: char| ('1'..='9').contains(&c)) && text.bytes().all(|b| b.is_ascii_digit());
    if !canonical {
        return Err(refuse(&text, CANONICAL, None));
    }
    text.parse::<NonZeroU32>()
        .map(FindingKey)
        .map_err(|source| refuse(&text, POSITIVE, Some(&source)))
}

impl<'de> Deserialize<'de> for FindingKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(FindingKeyVisitor)
    }
}

/// Deserializes a map keyed by finding numbers ([`NonZeroU32`]), accepting keys as integers or strings.
///
/// # Errors
/// Returns a deserialization error naming the key if a key is not a positive integer (e.g. `"0"`, `"-1"`, `"x"`, `""`).
pub(crate) fn deserialize_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<NonZeroU32, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    let map = BTreeMap::<FindingKey, V>::deserialize(deserializer)?;
    Ok(map.into_iter().map(|(key, value)| (key.0, value)).collect())
}

/// Deserializes a set of finding numbers ([`NonZeroU32`]), accepting elements as integers or strings.
///
/// # Errors
/// Returns a deserialization error naming the element if an element is not a positive integer.
pub(crate) fn deserialize_set<'de, D>(deserializer: D) -> Result<BTreeSet<NonZeroU32>, D::Error>
where
    D: Deserializer<'de>,
{
    let set = BTreeSet::<FindingKey>::deserialize(deserializer)?;
    Ok(set.into_iter().map(|key| key.0).collect())
}

#[cfg(test)]
mod tests;
