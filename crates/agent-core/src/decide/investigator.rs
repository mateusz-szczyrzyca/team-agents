//! One activity order for all investigator requests, with permanent and last-request exclusions.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use crate::ids::{InvestigatorName, TeamId};
use crate::kinds::ActivityKind;
use crate::state::Timestamp;

/// The assignment facts needed by the selector; outcomes do not affect rotation.
#[derive(Debug, Clone, Copy)]
pub struct Activity<'a> {
    /// Investigator that received the assignment.
    pub investigator: &'a InvestigatorName,
    /// Team that requested it.
    pub team: &'a TeamId,
    /// Classification, including permanent crew exclusion.
    pub kind: ActivityKind,
    /// Assignment time; equal times in a team form one request.
    pub asked_at: Timestamp,
}

/// Selects a complete request, or waits when too few free investigators remain.
///
/// Crew and caller exclusions are permanent. The latest request is placed after all
/// other candidates, then each group is ordered by last team activity, last global
/// activity and name. No activity sorts before any timestamp.
#[must_use]
pub fn choose(
    configured: &BTreeSet<InvestigatorName>,
    free: &BTreeSet<InvestigatorName>,
    activities: &[Activity<'_>],
    team: &TeamId,
    excluded: &BTreeSet<InvestigatorName>,
    required: NonZeroUsize,
) -> Option<Vec<InvestigatorName>> {
    let latest = activities
        .iter()
        .filter(|activity| activity.team == team)
        .map(|activity| activity.asked_at)
        .max();
    let mut candidates = Vec::new();
    for name in configured.intersection(free) {
        if excluded.contains(name) {
            continue;
        }
        let own: Vec<_> = activities
            .iter()
            .filter(|activity| activity.investigator == name)
            .collect();
        if own
            .iter()
            .any(|activity| activity.team == team && activity.kind == ActivityKind::TeamMember)
        {
            continue;
        }
        let last_team = own
            .iter()
            .filter(|activity| activity.team == team)
            .map(|activity| activity.asked_at)
            .max();
        let last_anywhere = own.iter().map(|activity| activity.asked_at).max();
        let recent = latest.is_some() && last_team == latest;
        candidates.push((recent, last_team, last_anywhere, name));
    }
    if candidates.len() < required.get() {
        return None;
    }
    candidates.sort_unstable();
    Some(
        candidates
            .into_iter()
            .take(required.get())
            .map(|(_, _, _, name)| name.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests;
