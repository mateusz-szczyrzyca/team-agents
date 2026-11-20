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
    /// The proposal did not read in its form, or a cycle has no part to name.
    Unreadable {
        /// Part with the fault, if identifiable.
        part: Option<PartKey>,
    },
    /// The first part depends on another part, so nothing can start the split.
    FirstPartDepends {
        /// The first part.
        part: PartKey,
    },
    /// A part depends on a part that is not in the proposal.
    UnknownDependency {
        /// Part naming the dependency.
        part: PartKey,
        /// The part that does not exist.
        dependency: PartKey,
    },
    /// A part names the same dependency twice.
    DuplicateDependency {
        /// Part naming the dependency.
        part: PartKey,
        /// The repeated dependency.
        dependency: PartKey,
    },
    /// A part has no title.
    EmptyTitle {
        /// Part without a title.
        part: PartKey,
    },
    /// A part does not say what it delivers.
    EmptyDeliverable {
        /// Part without a deliverable.
        part: PartKey,
    },
    /// The proposal has fewer than two or more than 26 parts.
    PartCount {
        /// Number of parts the proposal has.
        count: usize,
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
        return fault(ProposalFault::FirstPartDepends { part: id.key() }, policy, corrections);
    }
    for (key, part) in proposal.entries() {
        if let Some(estimate) = &part.estimate
            && policy.part_limit != 0
            && same_unit(estimate.unit(), unit)
            && estimate
                .amount()
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

/// Whether two units name the same thing: case, surrounding spaces and a plural `s` do not matter.
fn same_unit(left: &str, right: &str) -> bool {
    fn normal(unit: &str) -> String {
        let lower = unit.trim().to_lowercase();
        lower.strip_suffix('s').unwrap_or(&lower).to_owned()
    }
    normal(left) == normal(right)
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
        SplitError::Count { count } => ProposalFault::PartCount { count: *count },
        SplitError::Empty {
            part,
            field: PartField::Checks,
        } => ProposalFault::MissingChecks { part: *part },
        SplitError::Empty {
            part,
            field: PartField::Title,
        } => ProposalFault::EmptyTitle { part: *part },
        SplitError::Empty {
            part,
            field: PartField::Deliverable,
        } => ProposalFault::EmptyDeliverable { part: *part },
        SplitError::UnknownDependency { part, dependency } => ProposalFault::UnknownDependency {
            part: *part,
            dependency: *dependency,
        },
        SplitError::DuplicateDependency { part, dependency } => ProposalFault::DuplicateDependency {
            part: *part,
            dependency: *dependency,
        },
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
