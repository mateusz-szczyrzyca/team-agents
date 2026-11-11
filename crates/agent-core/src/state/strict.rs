//! Serde implementations without a public raw `deserialize`: checked values and strict internally tagged enums.

use std::fmt::Display;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Reads an enum through its private serde-generated implementation, then rejects input fields that its variant did
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

/// Defines a struct whose `Deserialize` applies `$validate` after reading.
///
/// The definition `$item` is emitted twice: as the public type, which derives only `Serialize`, and as a private copy
/// inside `raw` carrying the raw serde derive through `remote`. A derive on the public type would add a public inherent
/// `Type::deserialize` that wins over the trait function and skips the validation. `$serde` is the container's serde
/// attributes and `$remote` the path of the public type, as a literal because serde reads attributes literally.
macro_rules! checked {
    ($name:ident, $remote:literal, serde($($serde:tt)*), $validate:expr; $item:item) => {
        #[derive(serde::Serialize)]
        #[serde($($serde)*)]
        $item

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                fn raw<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                    #[derive(serde::Deserialize)]
                    #[expect(unreachable_pub, reason = "the copy repeats the public definition, which says `pub`")]
                    #[serde(remote = $remote, $($serde)*)]
                    $item
                    $name::deserialize(deserializer)
                }
                let value = raw(deserializer)?;
                $validate(&value).map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

/// Defines an internally tagged enum that refuses input fields its variant does not have (see [`read`]) and, like
/// [`checked!`], deserializes through a private copy of the definition. `omitted` and `validate` are optional.
macro_rules! strict_enum {
    ($name:ident, $remote:literal, serde($($serde:tt)*); $item:item) => {
        $crate::state::strict::strict_enum!($name, $remote, serde($($serde)*), |_| &[]; $item);
    };
    ($name:ident, $remote:literal, serde($($serde:tt)*), $omitted:expr; $item:item) => {
        $crate::state::strict::strict_enum!(
            $name, $remote, serde($($serde)*), $omitted, |_| Ok::<(), serde_json::Error>(()); $item
        );
    };
    ($name:ident, $remote:literal, serde($($serde:tt)*), $omitted:expr, $validate:expr; $item:item) => {
        #[derive(serde::Serialize)]
        #[serde($($serde)*)]
        $item

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                fn raw(value: serde_json::Value) -> Result<$name, serde_json::Error> {
                    #[derive(serde::Deserialize)]
                    #[expect(unreachable_pub, reason = "the copy repeats the public definition, which says `pub`")]
                    #[serde(remote = $remote, $($serde)*)]
                    $item
                    $name::deserialize(value)
                }
                $crate::state::strict::read(deserializer, raw, $omitted, $validate)
            }
        }
    };
}

pub(crate) use {checked, strict_enum};
