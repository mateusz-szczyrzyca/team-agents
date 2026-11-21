//! Human-readable progress of the parts of each approved split.
//!
//! Every view of the status prints the same state for a part: a view is the daemon's answer and, for the running view,
//! the same answer with its round lines limited. The state of a part is a decision on the whole team — the plan, the
//! rounds of its item, merged rounds included, and the wait for a tier — taken once and printed by every view.

use std::collections::BTreeSet;

use agent_core::protocol::TeamSummary;
use agent_core::state::RoundLife;

use super::View;
use agent_core::state::split::{PartId, PartKey, SplitPlan};

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
    WaitingFor(Vec<PartId>),
    /// The item waits for an agent of the tier it asks for before its next part starts.
    WaitingForTier,
}

/// The state of each part of every split of `team`, in the declaration order of the plan.
pub(super) fn states(team: &TeamSummary) -> Vec<(&SplitPlan, Vec<(PartId, PartState)>)> {
    team.splits.iter().map(|plan| (plan, states_of(plan, team))).collect()
}

/// The state of each part of `plan` with `merged`, from the rounds `team` carries and its wait for a tier.
///
/// A part is over when a round of the item bound it and closed: the round that merged its pull request, or one that
/// gave it up. A part whose round is open is being worked on, unless the item waits for an agent of a tier. Another
/// part is next when every part it depends on is over, and otherwise waits for those that are not.
fn states_of(plan: &SplitPlan, team: &TeamSummary) -> Vec<(PartId, PartState)> {
    let rounds: Vec<_> = team
        .rounds
        .iter()
        .filter(|round| round.item() == Some(plan.parent()))
        .collect();
    let merged: BTreeSet<PartKey> = rounds
        .iter()
        .filter(|round| round.part_pull_request().is_some())
        .filter_map(|round| round.part())
        .collect();
    let closed: BTreeSet<PartKey> = rounds
        .iter()
        .filter(|round| matches!(round.life(), RoundLife::Closed { .. }))
        .filter_map(|round| round.part())
        .collect();
    let open = rounds
        .iter()
        .find(|round| matches!(round.life(), RoundLife::Open))
        .and_then(|round| round.part());
    let tier_wait = team.tier_waits.contains_key(&plan.parent());
    plan.entries()
        .map(|(id, definition)| {
            let key = id.key();
            let state = if merged.contains(&key) || closed.contains(&key) && has_pull_request(&rounds, key) {
                PartState::Done
            } else if open == Some(key) {
                if tier_wait {
                    PartState::WaitingForTier
                } else {
                    PartState::InProgress
                }
            } else {
                let over =
                    |key: PartKey| merged.contains(&key) || (closed.contains(&key) && has_pull_request(&rounds, key));
                let waiting: Vec<PartId> = plan
                    .entries()
                    .filter(|(other, _)| definition.depends_on.contains(&other.key()) && !over(other.key()))
                    .map(|(other, _)| other)
                    .collect();
                if !waiting.is_empty() {
                    PartState::WaitingFor(waiting)
                } else if tier_wait {
                    PartState::WaitingForTier
                } else {
                    PartState::Ready
                }
            };
            (id, state)
        })
        .collect()
}

/// Prints each part of the full view.
#[cfg(test)]
pub(super) fn lines(team: &TeamSummary) -> Vec<String> {
    lines_of(team, View::Full)
}

/// Prints each part from the plan that the daemon schedules and the rounds the team carries, for `view`: the running
/// view leaves out the parts of an item whose every part merged, whose work is over.
pub(super) fn lines_of(team: &TeamSummary, view: View) -> Vec<String> {
    let mut lines = Vec::new();
    for (plan, parts) in states(team) {
        if view == View::Running && parts.iter().all(|(_, state)| *state == PartState::Done) {
            continue;
        }
        for (id, state) in parts {
            let Some((_, definition)) = plan.entries().find(|(other, _)| *other == id) else {
                continue;
            };
            let text = match &state {
                PartState::Done => "done".to_owned(),
                PartState::InProgress => "in progress".to_owned(),
                PartState::Ready => "to do".to_owned(),
                PartState::WaitingForTier => team
                    .tier_waits
                    .get(&plan.parent())
                    .map_or_else(|| "to do".to_owned(), |tier| format!("waiting for {tier} agent")),
                PartState::WaitingFor(waiting) => {
                    let names: Vec<String> = waiting.iter().map(std::string::ToString::to_string).collect();
                    format!("waiting for {}", names.join(", "))
                }
            };
            lines.push(format!("    {id}  {text}  {}", definition.title));
        }
    }
    lines
}

/// Whether a round of the item bound the part `key` and carried a pull request: the part merged.
fn has_pull_request(rounds: &[&agent_core::state::Round], key: PartKey) -> bool {
    rounds
        .iter()
        .any(|round| round.part() == Some(key) && round.part_pull_request().is_some())
}
