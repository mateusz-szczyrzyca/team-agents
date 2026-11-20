//! Validated plans for splitting a task-list item, independent of how its parts are executed.

pub mod keeping;
pub mod proposal;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ids::ItemId;

/// Supported part keys; their order defines stable names without changing task-list IDs.
const PART_KEYS: &str = "abcdefghijklmnopqrstuvwxyz";

/// Position of a part in its split, shown as a lowercase letter from `a` through `z`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PartKey(
    /// Validated single lowercase ASCII letter.
    char,
);

impl std::fmt::Display for PartKey {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(out)
    }
}

impl TryFrom<String> for PartKey {
    type Error = InvalidPartKey;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let mut chars = raw.chars();
        match (chars.next(), chars.next()) {
            (Some(letter), None) if letter.is_ascii_lowercase() => Ok(Self(letter)),
            _ => Err(InvalidPartKey { raw }),
        }
    }
}

impl From<PartKey> for String {
    fn from(key: PartKey) -> Self {
        key.0.to_string()
    }
}

/// Identity of one part, separate from the unchanged two-number task-list ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PartId {
    /// Original task-list item.
    parent: ItemId,
    /// Position within its split.
    key: PartKey,
}

impl PartId {
    getter!("The original item.", pub const fn parent(&self) -> ItemId = self.parent);
    getter!("The letter assigned to this part.", pub const fn key(&self) -> PartKey = self.key);
}

impl std::fmt::Display for PartId {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}{}", self.parent, self.key)
    }
}

/// A part key was not exactly one lowercase ASCII letter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid part key {raw:?}: use one letter from a through z")]
pub struct InvalidPartKey {
    /// The refused text.
    pub raw: String,
}

/// Unvalidated input for one part; its position in the plan assigns its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDefinition {
    /// A short title naming the deliverable.
    pub title: String,
    /// What this part delivers independently.
    pub delivers: String,
    /// Commands or criteria that establish delivery; not restricted to software tests.
    pub checks: String,
    /// Other parts that must be merged before this part starts.
    pub depends_on: Vec<PartKey>,
}

/// A whole validated split, with stable keys assigned in the given order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SplitWire", into = "SplitWire")]
pub struct SplitPlan {
    /// The original task-list item, whose syntax stays unchanged.
    parent: ItemId,
    /// At least two independently deliverable parts, at most 26.
    parts: Vec<PartDefinition>,
    /// Non-default requirements keyed by stable part identity.
    requirements: BTreeMap<PartKey, proposal::Requirements>,
}

/// Serialized plan input, validated through the same constructor as a human verdict.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SplitWire {
    /// The item being split.
    parent: ItemId,
    /// Parts in letter order, including optional requirements.
    parts: Vec<proposal::PartInput>,
}

impl TryFrom<SplitWire> for SplitPlan {
    type Error = SplitError;
    fn try_from(wire: SplitWire) -> Result<Self, Self::Error> {
        Self::from_input(wire.parent, &wire.parts)
    }
}

impl From<SplitPlan> for SplitWire {
    fn from(plan: SplitPlan) -> Self {
        let parts = plan
            .entries()
            .map(|(id, part)| {
                let requirements = plan.requirements.get(&id.key()).cloned().unwrap_or_default();
                proposal::PartInput {
                    title: part.title.clone(),
                    delivers: part.delivers.clone(),
                    checks: part.checks.clone(),
                    depends_on: part.depends_on.clone(),
                    estimate: requirements.estimate,
                    tier: requirements.tier,
                }
            })
            .collect();
        Self {
            parent: plan.parent,
            parts,
        }
    }
}

/// A split was incomplete or its dependencies were invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    /// The number of parts cannot be named by the supported keys.
    #[error("a split needs 2 through 26 parts, got {count}")]
    Count {
        /// Number supplied.
        count: usize,
    },
    /// One required field is blank.
    #[error("part {part} has no {field}")]
    Empty {
        /// The part with the missing text.
        part: PartKey,
        /// Which text is missing.
        field: PartField,
    },
    /// A dependency is outside this plan.
    #[error("part {part} depends on unknown part {dependency}")]
    UnknownDependency {
        /// The part naming the dependency.
        part: PartKey,
        /// The unknown part.
        dependency: PartKey,
    },
    /// A dependency was named twice.
    #[error("part {part} depends on part {dependency} twice")]
    DuplicateDependency {
        /// The part naming the dependency.
        part: PartKey,
        /// The repeated part.
        dependency: PartKey,
    },
    /// The graph contains a cycle, including a self dependency.
    #[error("split dependencies contain a cycle")]
    Cycle,
}

/// A required textual field of a split part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartField {
    /// The short title.
    Title,
    /// The deliverable description.
    Deliverable,
    /// The acceptance criteria.
    Checks,
}

impl std::fmt::Display for PartField {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Title => "title",
            Self::Deliverable => "deliverable",
            Self::Checks => "checks",
        })
    }
}

impl SplitPlan {
    /// Validates a complete split before any part can be recorded.
    ///
    /// # Errors
    /// [`SplitError`] for missing text, an invalid part count, repeated or unknown dependencies, or a cycle.
    pub fn new(parent: ItemId, parts: Vec<PartDefinition>) -> Result<Self, SplitError> {
        if !(2..=PART_KEYS.len()).contains(&parts.len()) {
            return Err(SplitError::Count { count: parts.len() });
        }
        let plan = Self {
            parent,
            parts,
            requirements: BTreeMap::new(),
        };
        let known: BTreeSet<PartKey> = plan.entries().map(|(id, _)| id.key).collect();
        for (id, part) in plan.entries() {
            for (field, text) in [
                (PartField::Title, &part.title),
                (PartField::Deliverable, &part.delivers),
                (PartField::Checks, &part.checks),
            ] {
                if text.trim().is_empty() {
                    return Err(SplitError::Empty { part: id.key, field });
                }
            }
            let mut seen = BTreeSet::new();
            for dependency in &part.depends_on {
                if !known.contains(dependency) {
                    return Err(SplitError::UnknownDependency {
                        part: id.key,
                        dependency: *dependency,
                    });
                }
                if !seen.insert(*dependency) {
                    return Err(SplitError::DuplicateDependency {
                        part: id.key,
                        dependency: *dependency,
                    });
                }
            }
        }
        plan.check_acyclic()?;
        Ok(plan)
    }

    /// Resolves at most 26 nodes iteratively; forward references are allowed but cycles never are.
    fn check_acyclic(&self) -> Result<(), SplitError> {
        let mut resolved = BTreeSet::new();
        loop {
            let before = resolved.len();
            for (id, part) in self.entries() {
                if part.depends_on.iter().all(|key| resolved.contains(key)) {
                    resolved.insert(id.key);
                }
            }
            if resolved.len() == self.parts.len() {
                return Ok(());
            }
            if resolved.len() == before {
                return Err(SplitError::Cycle);
            }
        }
    }

    /// Parts paired with their stable identities in declaration order.
    pub fn entries(&self) -> impl Iterator<Item = (PartId, &PartDefinition)> {
        self.parts.iter().zip(PART_KEYS.chars()).map(|(part, letter)| {
            (
                PartId {
                    parent: self.parent,
                    key: PartKey(letter),
                },
                part,
            )
        })
    }

    /// Requested agent tier and estimate of an existing part; unknown keys return none.
    #[must_use]
    pub fn requirements(&self, key: PartKey) -> Option<proposal::Requirements> {
        self.entries()
            .find(|(id, _)| id.key() == key)
            .map(|_| self.requirements.get(&key).cloned().unwrap_or_default())
    }

    /// Validates definitions and retains only requirements absent from legacy plans.
    pub(crate) fn from_input(parent: ItemId, parts: &[proposal::PartInput]) -> Result<Self, SplitError> {
        let mut plan = Self::new(parent, parts.iter().map(proposal::PartInput::definition).collect())?;
        let requirements = parts
            .iter()
            .zip(PART_KEYS.chars())
            .map(|(part, key)| (PartKey(key), part.requirements()))
            .filter(|(_, requirements)| requirements != &proposal::Requirements::default())
            .collect();
        plan.requirements = requirements;
        Ok(plan)
    }

    getter!("The original item.", pub const fn parent(&self) -> ItemId = self.parent);
    getter!("Validated parts in their declared order.", pub fn parts(&self) -> &[PartDefinition] = &self.parts);
}

#[cfg(test)]
mod tests;
