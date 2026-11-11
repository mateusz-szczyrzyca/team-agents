//! The pure core of the agent harness: identifiers ([`ids`]), kinds ([`kinds`]), statuses ([`status`]), the
//! configuration ([`config`]), the task list ([`task_list`]), the questions files ([`questions`]), the
//! state ([`state`]), the control protocol ([`protocol`]) and, later, the decisions (see `docs/architecture.md`).
//!
//! Nothing here does I/O: the clock and the random source are passed in ([`ids::Clock`], [`rand::Rng`]), so every
//! function is deterministic under test.
#![forbid(unsafe_code)]

/// Defines a documented zero-argument getter and its `#[must_use]` attribute from its returned expression.
macro_rules! getter {
    ($doc:literal, $vis:vis const fn $name:ident(&$this:ident) -> $ret:ty = $expr:expr) => {
        #[doc = $doc]
        #[must_use]
        $vis const fn $name(&$this) -> $ret {
            $expr
        }
    };
    ($doc:literal, $vis:vis fn $name:ident(&$this:ident) -> $ret:ty = $expr:expr) => {
        #[doc = $doc]
        #[must_use]
        $vis fn $name(&$this) -> $ret {
            $expr
        }
    };
}

/// Implements the serde of a unit version type, refusing any other number with its operation-specific error.
macro_rules! version_type {
    ($name:ident, $version:ident, $error:ident) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_u32($version)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let found = u64::deserialize(deserializer)?;
                if found == u64::from($version) {
                    Ok(Self)
                } else {
                    Err(serde::de::Error::custom($error { found }))
                }
            }
        }
    };
}

pub mod config;
pub mod ids;
pub mod kinds;
pub mod protocol;
pub mod questions;
pub mod state;
pub mod status;
pub mod task_list;
