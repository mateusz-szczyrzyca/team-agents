//! Strict deserialization for internally tagged state enums.

use std::fmt::Display;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Reads an enum through its serde-generated remote implementation, then rejects input fields that its variant did
/// not serialize. `omitted` names valid optional fields that serialization omits; `validate` applies domain checks.
pub(crate) fn read<'de, D, T, F, O, V, E>(deserializer: D, parse: F, omitted: O, validate: V) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Serialize,
    F: FnOnce(Value) -> Result<T, serde_json::Error>,
    O: FnOnce(&T) -> &'static [&'static str],
    V: FnOnce(&T) -> Result<(), E>,
    E: Display,
{
    let value = Value::deserialize(deserializer)?;
    let parsed = parse(value.clone()).map_err(serde::de::Error::custom)?;
    validate(&parsed).map_err(serde::de::Error::custom)?;
    let known = serde_json::to_value(&parsed).map_err(serde::de::Error::custom)?;
    let input = value
        .as_object()
        .ok_or_else(|| serde::de::Error::custom("an internally tagged enum must be a JSON object"))?;
    let known = known
        .as_object()
        .ok_or_else(|| serde::de::Error::custom("an internally tagged enum must serialize as a JSON object"))?;
    let omitted = omitted(&parsed);
    if let Some(field) = input
        .keys()
        .find(|field| !known.contains_key(*field) && !omitted.contains(&field.as_str()))
    {
        return Err(serde::de::Error::custom(format!("unknown field `{field}`")));
    }
    Ok(parsed)
}

/// Implements the ordinary serde traits through a `remote = "Self"` derivation and strict input-field checking.
macro_rules! strict_enum {
    ($name:ident) => {
        $crate::state::strict::strict_enum!($name, |_| &[]);
    };
    ($name:ident, $omitted:expr) => {
        $crate::state::strict::strict_enum!($name, $omitted, |_| Ok::<(), serde_json::Error>(()));
    };
    ($name:ident, $omitted:expr, $validate:expr) => {
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                Self::serialize(self, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                $crate::state::strict::read(deserializer, Self::deserialize, $omitted, $validate)
            }
        }
    };
}

pub(crate) use strict_enum;
