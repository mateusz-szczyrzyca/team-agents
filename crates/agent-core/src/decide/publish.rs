//! Publication decisions: why a finished branch is not published yet.
//!
//! A branch of a lead is published — pushed and a pull request opened or updated — only when the lead's recorded
//! gate passed on a clean tree at the branch's tip. Until then, [`publication_wait_reason`] returns the reason why
//! publication waits:
//!
//! - [`PublicationWaitReason::NoBranchTip`]: the branch has no tip commit.
//! - [`PublicationWaitReason::NoGateRecord`]: the lead's task has no gate record.
//! - [`PublicationWaitReason::GateAtAnotherCommit`]: the gate ran on a commit that is not the branch tip.
//! - [`PublicationWaitReason::GateFailed`]: the gate command was not a pass (it failed or timed out).
//! - [`PublicationWaitReason::GateTreeNotClean`]: the gate passed on a tree that was not clean.
//!
//! When none of these holds, the branch is ready for publication and [`publication_wait_reason`] returns `None`.

use serde::{Deserialize, Serialize};

use crate::state::{CommitId, GateOutcome, GateRecord, GateTree};

/// Why a finished branch is not published yet.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[strum(serialize_all = "snake_case")]
pub enum PublicationWaitReason {
    /// The branch has no tip commit.
    NoBranchTip,
    /// The lead's task has no gate record.
    NoGateRecord,
    /// The gate ran on a commit that is not the branch tip.
    GateAtAnotherCommit,
    /// The gate command was not a pass (it failed or timed out).
    GateFailed,
    /// The gate passed on a tree that was not clean.
    GateTreeNotClean,
}

impl PublicationWaitReason {
    /// The `snake_case` keyword of this reason.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Decides why a branch with `tip` and recorded `record` cannot be published yet, or `None` if it is ready.
///
/// Order of precedence when multiple reasons apply:
/// 1. Missing branch tip ([`PublicationWaitReason::NoBranchTip`]).
/// 2. Missing gate record ([`PublicationWaitReason::NoGateRecord`]).
/// 3. Gate at another commit ([`PublicationWaitReason::GateAtAnotherCommit`]), even if that gate failed.
/// 4. Gate failed or timed out ([`PublicationWaitReason::GateFailed`]).
/// 5. Gate passed on dirty tree ([`PublicationWaitReason::GateTreeNotClean`]).
#[must_use]
pub fn publication_wait_reason(tip: Option<&CommitId>, record: Option<&GateRecord>) -> Option<PublicationWaitReason> {
    let Some(tip) = tip else {
        return Some(PublicationWaitReason::NoBranchTip);
    };
    let Some(record) = record else {
        return Some(PublicationWaitReason::NoGateRecord);
    };
    if record.result.head != *tip {
        return Some(PublicationWaitReason::GateAtAnotherCommit);
    }
    if record.result.outcome != GateOutcome::Passed {
        return Some(PublicationWaitReason::GateFailed);
    }
    if record.tree != GateTree::Clean {
        return Some(PublicationWaitReason::GateTreeNotClean);
    }
    None
}

#[cfg(test)]
mod tests;
