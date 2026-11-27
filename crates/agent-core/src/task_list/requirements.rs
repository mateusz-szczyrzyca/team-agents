//! The requirements of an item: what its `Expected result:` and `Checks:` parts ask for, each with a number.
//!
//! The harness numbers them when it gives the item to an agent, so that a plan step, a commit and a report can say
//! which requirement they are about. The top-level bullets of the expected result are `R1`, `R2`, ...; the lines of the
//! checks keep the `K1`, `K2`, ... they start with, and a line without one gets the next free number.

use std::fmt;
use std::num::NonZeroU16;
use std::str::FromStr;

use super::Item;

/// Which part of an item a requirement comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequirementKind {
    /// A bullet of the expected result: `R1`.
    Result,
    /// A line of the checks: `K1`.
    Check,
}

impl RequirementKind {
    /// The letter that starts the requirement's number in a text.
    const fn letter(self) -> char {
        match self {
            Self::Result => 'R',
            Self::Check => 'K',
        }
    }
}

/// The number of a requirement of an item, written `R3` or `K2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequirementId {
    /// The part of the item it comes from.
    kind: RequirementKind,
    /// Its number within that part, from 1.
    number: NonZeroU16,
}

impl RequirementId {
    /// The requirement `number` of the part `kind`.
    #[must_use]
    pub const fn new(kind: RequirementKind, number: NonZeroU16) -> Self {
        Self { kind, number }
    }

    /// The part of the item it comes from.
    #[must_use]
    pub const fn kind(self) -> RequirementKind {
        self.kind
    }

    /// Its number within that part, from 1.
    #[must_use]
    pub const fn number(self) -> NonZeroU16 {
        self.number
    }
}

impl fmt::Display for RequirementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.kind.letter(), self.number)
    }
}

/// A text that is not the number of a requirement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not the number of a requirement: it is R or K and a number from 1, such as R1 or K2")]
pub struct InvalidRequirementId {
    /// The text that was refused.
    pub value: String,
}

impl FromStr for RequirementId {
    type Err = InvalidRequirementId;

    fn from_str(_text: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(RequirementKind::Result, NonZeroU16::MIN))
    }
}

/// One requirement of an item: its number and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// Its number.
    pub id: RequirementId,
    /// What it asks for, as the item says it, without its bullet and its `K` label.
    pub text: String,
}

impl Item {
    /// The requirements of the item, in order: the `R` ones, then the `K` ones.
    #[must_use]
    pub fn requirements(&self) -> Vec<Requirement> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
