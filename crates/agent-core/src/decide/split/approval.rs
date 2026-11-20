//! Pure approval policy; callers provide a repository unit without assuming code deliverables.

use crate::config::Split;
use crate::state::split::{
    PartField, PartKey, SplitError, SplitPlan,
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
pub fn decide_proposal(read: ProposalRead<'_>, policy: Split, unit: &str, corrections: u32) -> ProposalDecision {
    let proposal = match read {
        ProposalRead::Parsed(proposal) => proposal,
        ProposalRead::Unreadable { part } => return fault(ProposalFault::Unreadable { part }, policy, corrections),
    };
    let plan = match proposal.plan() {
        Ok(plan) => plan,
        Err(error) => return fault(plan_fault(proposal, &error), policy, corrections),
    };
    if let Some((id, first)) = plan.entries().next()
        && !first.depends_on.is_empty()
    {
        return fault(ProposalFault::Unreadable { part: Some(id.key()) }, policy, corrections);
    }
    for (key, part) in proposal.entries() {
        if let Some(estimate) = &part.estimate
            && policy.part_limit != 0
            && estimate.unit == unit
            && estimate
                .amount
                .as_f64()
                .is_some_and(|amount| amount > f64::from(policy.part_limit))
        {
            return fault(
                ProposalFault::AboveLimit {
                    part: key,
                    estimate: estimate.clone(),
                    limit: policy.part_limit,
                },
                policy,
                corrections,
            );
        }
    }
    match proposal.scope() {
        ScopeChange::Unchanged => ProposalDecision::Approve(plan),
        change @ (ScopeChange::Less { .. } | ScopeChange::More { .. } | ScopeChange::Different { .. }) => {
            ProposalDecision::AskHuman(AskReason::ScopeChanged(change.clone()))
        }
    }
}

fn fault(fault: ProposalFault, policy: Split, corrections: u32) -> ProposalDecision {
    if corrections < policy.corrections {
        ProposalDecision::SendBack(fault)
    } else {
        ProposalDecision::AskHuman(AskReason::CorrectionsUsed(fault))
    }
}

fn plan_fault(proposal: &Proposal, error: &SplitError) -> ProposalFault {
    match error {
        SplitError::Count { .. } => ProposalFault::Unreadable { part: None },
        SplitError::Empty {
            part,
            field: PartField::Checks,
        } => ProposalFault::MissingChecks { part: *part },
        SplitError::Empty {
            part,
            field: PartField::Title | PartField::Deliverable,
        }
        | SplitError::UnknownDependency { part, .. }
        | SplitError::DuplicateDependency { part, .. } => ProposalFault::Unreadable { part: Some(*part) },
        SplitError::Cycle => match cycle_member(proposal) {
            Some(part) => ProposalFault::Cycle { part },
            // Preserve rejection even if a future plan validator reports a cycle with no witness.
            None => ProposalFault::Unreadable { part: None },
        },
    }
}

/// Search from each bounded node for a path back to itself, naming a cycle member, not a dependant.
fn cycle_member(proposal: &Proposal) -> Option<PartKey> {
    for (start, part) in proposal.entries() {
        let mut pending = part.depends_on.clone();
        let mut visited = std::collections::BTreeSet::new();
        while let Some(key) = pending.pop() {
            if key == start {
                return Some(start);
            }
            if visited.insert(key)
                && let Some((_, next)) = proposal.entries().find(|(candidate, _)| *candidate == key)
            {
                pending.extend(&next.depends_on);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
