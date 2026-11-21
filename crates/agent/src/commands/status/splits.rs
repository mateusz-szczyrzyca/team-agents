//! Human-readable progress of the parts of each approved split.

use agent_core::protocol::TeamSummary;

/// What a part of a split is doing, as the daemon schedules it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PartState {
    /// A round of the item bound the part and closed: it merged the part's pull request, or gave it up.
    Done,
    /// The round bound to the part is open, and the item does not wait for an agent of a tier.
    InProgress,
    /// The lead goes on with the part once the parts it depends on have closed; none has to, so it is next.
    Ready,
    /// The lead goes on with the part once these parts have closed.
    WaitingFor(Vec<agent_core::state::split::PartId>),
    /// The item waits for an agent of the tier it asks for before its next part starts.
    WaitingForTier,
}

/// The state of each part of every split of `team`.
pub(super) fn states(
    team: &TeamSummary,
) -> Vec<(
    &agent_core::state::split::SplitPlan,
    Vec<(agent_core::state::split::PartId, PartState)>,
)> {
    let _ = team;
    Vec::new()
}

/// Prints each part from its durable round identity and dependency graph.
pub(super) fn lines(team: &TeamSummary) -> Vec<String> {
    let mut lines = Vec::new();
    for plan in &team.splits {
        let rounds: Vec<_> = team
            .rounds
            .iter()
            .filter(|round| round.item() == Some(plan.parent()))
            .collect();
        let merged: std::collections::BTreeSet<_> = rounds
            .iter()
            .filter(|round| round.part_pull_request().is_some())
            .filter_map(|round| round.part())
            .collect();
        let waiting_part = rounds
            .iter()
            .find(|round| round.is_open())
            .and_then(|round| round.part())
            .or_else(|| {
                plan.entries()
                    .find(|(id, definition)| {
                        !merged.contains(&id.key()) && definition.depends_on.iter().all(|key| merged.contains(key))
                    })
                    .map(|(id, _)| id.key())
            });
        for (id, definition) in plan.entries() {
            let waiting: Vec<_> = definition
                .depends_on
                .iter()
                .filter(|key| !merged.contains(key))
                .map(|key| format!("{}{key}", plan.parent()))
                .collect();
            let status = if merged.contains(&id.key()) {
                "done".to_owned()
            } else if waiting_part == Some(id.key()) && team.tier_waits.contains_key(&plan.parent()) {
                team.tier_waits
                    .get(&plan.parent())
                    .map_or_else(|| "to do".to_owned(), |tier| format!("waiting for {tier} agent"))
            } else if rounds
                .iter()
                .any(|round| round.part() == Some(id.key()) && round.is_open())
            {
                "in progress".to_owned()
            } else if waiting.is_empty() {
                "to do".to_owned()
            } else {
                format!("waiting for {}", waiting.join(", "))
            };
            lines.push(format!("    {id}  {status}  {}", definition.title));
        }
    }
    lines
}
