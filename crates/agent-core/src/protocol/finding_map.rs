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

/// A finding number parsed from either a string or an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FindingKey(NonZeroU32);

impl FindingKey {
    /// Returns the underlying [`NonZeroU32`].
    #[must_use]
    pub const fn get(self) -> NonZeroU32 {
        self.0
    }
}

/// Visitor that parses finding numbers from strings or integer types.
struct FindingKeyVisitor;

impl serde::de::Visitor<'_> for FindingKeyVisitor {
    type Value = FindingKey;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a positive integer finding number")
    }

    fn visit_str<E>(self, text: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        parse_finding_number(text)
    }

    fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        NonZeroU32::new(u32::from(v))
            .map(FindingKey)
            .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer")))
    }

    fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        NonZeroU32::new(u32::from(v))
            .map(FindingKey)
            .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer")))
    }

    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        NonZeroU32::new(v)
            .map(FindingKey)
            .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer")))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u32::try_from(v) {
            Ok(u) => NonZeroU32::new(u)
                .map(FindingKey)
                .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer"))),
            Err(source) => Err(E::custom(format!(
                "invalid finding number {v:?}: expected a positive integer: {source}"
            ))),
        }
    }

    fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u32::try_from(v) {
            Ok(u) => NonZeroU32::new(u)
                .map(FindingKey)
                .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer"))),
            Err(source) => Err(E::custom(format!(
                "invalid finding number {v:?}: expected a positive integer: {source}"
            ))),
        }
    }

    fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u32::try_from(v) {
            Ok(u) => NonZeroU32::new(u)
                .map(FindingKey)
                .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer"))),
            Err(source) => Err(E::custom(format!(
                "invalid finding number {v:?}: expected a positive integer: {source}"
            ))),
        }
    }

    fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u32::try_from(v) {
            Ok(u) => NonZeroU32::new(u)
                .map(FindingKey)
                .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer"))),
            Err(source) => Err(E::custom(format!(
                "invalid finding number {v:?}: expected a positive integer: {source}"
            ))),
        }
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u32::try_from(v) {
            Ok(u) => NonZeroU32::new(u)
                .map(FindingKey)
                .ok_or_else(|| E::custom(format!("invalid finding number {v:?}: expected a positive integer"))),
            Err(source) => Err(E::custom(format!(
                "invalid finding number {v:?}: expected a positive integer: {source}"
            ))),
        }
    }
}

/// Parses a finding number string into [`FindingKey`].
fn parse_finding_number<E: serde::de::Error>(text: &str) -> Result<FindingKey, E> {
    text.parse::<NonZeroU32>().map(FindingKey).map_err(|source| {
        E::custom(format!(
            "invalid finding number {text:?}: expected a positive integer: {source}"
        ))
    })
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
pub fn deserialize_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<NonZeroU32, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    let map = BTreeMap::<FindingKey, V>::deserialize(deserializer)?;
    Ok(map.into_iter().map(|(k, v)| (k.0, v)).collect())
}

/// Deserializes a set of finding numbers ([`NonZeroU32`]), accepting elements as integers or strings.
///
/// # Errors
/// Returns a deserialization error naming the element if an element is not a positive integer.
pub fn deserialize_set<'de, D>(deserializer: D) -> Result<BTreeSet<NonZeroU32>, D::Error>
where
    D: Deserializer<'de>,
{
    let set = BTreeSet::<FindingKey>::deserialize(deserializer)?;
    Ok(set.into_iter().map(|k| k.0).collect())
}
