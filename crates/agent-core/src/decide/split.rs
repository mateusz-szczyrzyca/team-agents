//! Dependency decisions for a validated split, without opening rounds or performing I/O.

use std::collections::BTreeSet;

use crate::state::split::{PartDefinition, PartId, PartKey, SplitPlan};

/// The next scheduling decision, in the declaration order of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextParts {
    /// Parts whose dependencies have all merged; the caller chooses assignments.
    Ready(Vec<PartId>),
    /// All parts have merged; the original item can close.
    Complete,
}

/// Progress refers to a part outside the approved plan.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("merged progress refers to unknown part {part}")]
pub struct UnknownMergedPart {
    /// The key absent from the plan.
    pub part: PartKey,
}

/// Returns all unmerged parts whose dependencies have merged, or completion.
///
/// Call between rounds: ongoing assignments remain the caller's responsibility.
///
/// # Errors
/// Returns [`UnknownMergedPart`] when progress contains a key outside this plan.
pub fn next_parts(plan: &SplitPlan, merged: &BTreeSet<PartKey>) -> Result<NextParts, UnknownMergedPart> {
    let known: BTreeSet<_> = plan.entries().map(|(id, _)| id.key()).collect();
    if let Some(part) = merged.difference(&known).next() {
        return Err(UnknownMergedPart { part: *part });
    }
    if merged.len() == known.len() {
        return Ok(NextParts::Complete);
    }
    let ready = ready_entries(plan, merged).map(|(id, _)| id).collect();
    Ok(NextParts::Ready(ready))
}

/// Lists dependency-ready definitions after the caller has checked the merged keys against the plan.
pub(crate) fn ready_entries<'plan, 'merged>(
    plan: &'plan SplitPlan,
    merged: &'merged BTreeSet<PartKey>,
) -> impl Iterator<Item = (PartId, &'plan PartDefinition)> + use<'plan, 'merged> {
    plan.entries()
        .filter(|(id, part)| !merged.contains(&id.key()) && part.depends_on.iter().all(|key| merged.contains(key)))
}

#[cfg(test)]
mod tests;
