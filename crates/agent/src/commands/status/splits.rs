//! Human-readable progress of the parts of each approved split.

use agent_core::protocol::TeamSummary;

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
        for (id, definition) in plan.entries() {
            let waiting: Vec<_> = definition
                .depends_on
                .iter()
                .filter(|key| !merged.contains(key))
                .map(|key| format!("{}{key}", plan.parent()))
                .collect();
            let status = if merged.contains(&id.key()) {
                "done".to_owned()
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
