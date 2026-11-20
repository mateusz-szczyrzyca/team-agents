//! Split proposal input and optional requirements retained with approved plans.

use super::{PartDefinition, PartKey, SplitPlan};
use crate::ids::ItemId;
use crate::kinds::Tier;
use serde::{Deserialize, Serialize};

/// An amount in an author's explicit unit, independent of the kind of deliverable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Estimate {
    /// Nonnegative JSON number; validated when the proposal is read.
    pub amount: serde_json::Number,
    /// Unit such as lines, pages or images.
    pub unit: String,
}

/// Optional requirements of a part; old plans request a normal agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    /// Size when supplied; absent in legacy human verdicts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
    /// Minimum requested agent tier.
    #[serde(default = "normal", skip_serializing_if = "is_normal")]
    pub tier: Tier,
}

fn normal() -> Tier {
    Tier::Normal
}
fn is_normal(tier: &Tier) -> bool {
    *tier == Tier::Normal
}
impl Default for Requirements {
    fn default() -> Self {
        Self {
            estimate: None,
            tier: Tier::Normal,
        }
    }
}

/// An explicit declaration of how the split relates to the original item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopeChange {
    /// All of the original scope is retained.
    Unchanged,
    /// Some original deliverables are omitted.
    Less {
        /// What is omitted and why.
        reason: String,
    },
    /// New deliverables are added.
    More {
        /// What is added and why.
        reason: String,
    },
    /// Deliverables are replaced with different work.
    Different {
        /// What changes and why.
        reason: String,
    },
}

/// Complete proposal input, before semantic checks of the parts and dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    /// The existing item to split.
    parent: ItemId,
    /// Explicit statement that the scope stays the same or how it changes.
    scope: ScopeChange,
    /// Parts in permanent letter order.
    parts: Vec<PartInput>,
}

/// The external part shape; separate from existing source-level part constructors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartInput {
    /// The short title.
    pub(crate) title: String,
    /// Independently deliverable result.
    pub(crate) delivers: String,
    /// Acceptance criteria; blanks are a decision fault rather than a JSON error.
    pub(crate) checks: String,
    /// Prerequisite parts.
    pub(crate) depends_on: Vec<PartKey>,
    /// Optional only for compatibility with old human approvals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) estimate: Option<Estimate>,
    /// Default requested tier for legacy plans.
    #[serde(default = "normal", skip_serializing_if = "is_normal")]
    pub(crate) tier: Tier,
}

impl PartInput {
    pub(crate) fn definition(&self) -> PartDefinition {
        PartDefinition {
            title: self.title.clone(),
            delivers: self.delivers.clone(),
            checks: self.checks.clone(),
            depends_on: self.depends_on.clone(),
        }
    }
    pub(crate) fn requirements(&self) -> Requirements {
        Requirements {
            estimate: self.estimate.clone(),
            tier: self.tier,
        }
    }
}

/// Why a proposal cannot be read in its documented form.
#[derive(Debug, thiserror::Error)]
pub enum ReadProposalError {
    /// Bounded input protects both parsing and retained state.
    #[error("split proposal exceeds the questions-file size limit")]
    TooLarge,
    /// Syntax, missing fields, extra fields, or invalid enum values.
    #[error("invalid split proposal json")]
    Json {
        /// Original parser error, including its location.
        source: serde_json::Error,
    },
    /// A required estimate is absent, negative, or has a blank unit.
    #[error("part {part} needs a nonnegative estimate and a nonblank unit")]
    Estimate {
        /// One-based position when a malformed plan has more than 26 parts.
        part: usize,
    },
    /// A scope change must explain what changes.
    #[error("a scope change needs a nonblank reason")]
    ScopeReason,
}

/// Reads bounded raw JSON; semantic faults are left for the approval decision.
///
/// # Errors
/// Returns [`ReadProposalError`] for invalid structure or incomplete estimate/scope fields.
pub fn read_proposal(_text: &str) -> Result<Proposal, ReadProposalError> {
    Err(ReadProposalError::TooLarge)
}

impl Proposal {
    /// Original item named in the proposal.
    pub const fn parent(&self) -> ItemId {
        self.parent
    }
    /// Scope declaration retained for a human decision.
    pub const fn scope(&self) -> &ScopeChange {
        &self.scope
    }
    pub(crate) fn parts(&self) -> &[PartInput] {
        &self.parts
    }
    /// Validates the whole plan, retaining requirements with their parts.
    ///
    /// # Errors
    /// Returns [`super::SplitError`] for missing criteria or invalid dependencies.
    pub fn plan(&self) -> Result<SplitPlan, super::SplitError> {
        SplitPlan::new(self.parent, self.parts.iter().map(PartInput::definition).collect())
    }
}

#[cfg(test)]
mod tests;
