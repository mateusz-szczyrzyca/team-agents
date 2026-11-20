//! Pure approval policy; callers provide a repository unit without assuming code deliverables.

use crate::config::Split;
use crate::state::split::{
    PartKey, SplitPlan,
    proposal::{Estimate, Proposal, ScopeChange},
};

/// What reading the coordinator's proposal produced; diagnostics stay with the caller.
#[derive(Debug, Clone, Copy)]
pub enum ProposalRead<'a> {
    /// Structurally readable input, not yet validated as a whole plan.
    Parsed(&'a Proposal),
    /// Unreadable input; a part is named when its identity can be recovered.
    Unreadable {
        /// Part with the bad input, if identifiable.
        part: Option<PartKey>,
    },
}

/// A fault the coordinator must correct before harness approval.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalFault {
    /// Syntax or other form requirements, including blocked first parts.
    Unreadable {
        /// Part with the fault, if identifiable.
        part: Option<PartKey>,
    },
    /// A part has no acceptance criteria.
    MissingChecks {
        /// Part requiring criteria.
        part: PartKey,
    },
    /// A cycle contains this part, including a self dependency.
    Cycle {
        /// A member of the dependency cycle.
        part: PartKey,
    },
    /// The part's estimate in the repository unit exceeds the policy.
    AboveLimit {
        /// Part above the limit.
        part: PartKey,
        /// Estimate the author supplied.
        estimate: Estimate,
        /// Configured maximum in the same unit.
        limit: u32,
    },
}

/// Why a proposal needs a human verdict rather than another correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskReason {
    /// The split changes the deliverables of the original item.
    ScopeChanged(ScopeChange),
    /// The allowed corrections are exhausted, retaining the failed condition.
    CorrectionsUsed(ProposalFault),
}

/// A whole approval, one correction, or one human decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalDecision {
    /// Record the complete validated plan atomically.
    Approve(SplitPlan),
    /// Send the fault back to the coordinator.
    SendBack(ProposalFault),
    /// Ask the human once; persisted state prevents repeated questions.
    AskHuman(AskReason),
}

/// Decides whether a proposal is safe to approve without consulting the human.
///
/// `corrections` counts completed send-backs; limits apply only to estimates in `unit`.
#[must_use]
pub fn decide_proposal(_read: ProposalRead<'_>, _policy: Split, _unit: &str, _corrections: u32) -> ProposalDecision {
    ProposalDecision::SendBack(ProposalFault::Unreadable { part: None })
}

#[cfg(test)]
mod tests;
