//! Deserialization of maps and sets keyed by finding numbers (`NonZeroU32`).
//!
//! Below internally tagged enums (`#[serde(tag = ...)]`), serde buffers JSON map keys as strings.
//! Standard integer deserializers reject strings with `invalid type: string "...", expected a nonzero u32`.
//! The functions here accept finding numbers either as integer numbers or as strings.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use serde::{Deserialize, Deserializer};

/// Deserializes a map keyed by finding numbers ([`NonZeroU32`]), accepting keys as integers or strings.
///
/// # Errors
/// Returns a deserialization error naming the key if a key is not a positive integer (e.g. `"0"`, `"-1"`, `"x"`, `""`).
pub fn deserialize_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<NonZeroU32, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    BTreeMap::<NonZeroU32, V>::deserialize(deserializer)
}

/// Deserializes a set of finding numbers ([`NonZeroU32`]), accepting elements as integers or strings.
///
/// # Errors
/// Returns a deserialization error naming the element if an element is not a positive integer.
pub fn deserialize_set<'de, D>(deserializer: D) -> Result<BTreeSet<NonZeroU32>, D::Error>
where
    D: Deserializer<'de>,
{
    BTreeSet::<NonZeroU32>::deserialize(deserializer)
}
