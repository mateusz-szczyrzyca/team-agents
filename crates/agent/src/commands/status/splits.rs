//! Human-readable progress of the parts of each approved split.
//!
//! Every view of the status prints the same state for a part: a view is the daemon's answer and, for the running view,
//! the same answer with its round lines limited. The state of a part is a decision on the whole team — the plan, the
//! rounds of its item, merged rounds included, and the wait for a tier — taken once and printed by every view.

use agent_core::decide::split::merged_parts;
use agent_core::kinds::Tier;
use agent_core::protocol::TeamSummary;
use agent_core::state::split::{PartDefinition, PartId, SplitPlan};
use agent_core::state::{Round, RoundLife};

use super::View;

/// What a part of a split is doing, as the daemon schedules it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PartState {
    /// The part's pull request merged.
    Done,
    /// The round bound to the part is open, and the item does not wait for an agent of a tier.
    InProgress,
    /// The parts it depends on merged, or it depends on none, so the lead may go on with it.
    Ready,
    /// The lead goes on with the part once these parts merged.
    WaitingFor(Vec<PartId>),
    /// The part is the next one and its item waits for an agent of the tier it asks for.
    WaitingForTier {
        /// The tier of the agent that is missing.
        tier: Tier,
    },
}

/// The parts of one split, each with its state, in the declaration order of the plan.
type PartStates = Vec<(PartId, PartState)>;

/// The state of each part of every split of `team`, in the declaration order of the plan.
pub(super) fn states(team: &TeamSummary) -> Vec<(&SplitPlan, PartStates)> {
    team.splits.iter().map(|plan| (plan, states_of(plan, team))).collect()
}

/// The state of each part of `plan`, from the rounds `team` carries and its wait for a tier.
///
/// A part is done when its pull request merged, as [`merged_parts`] tells: a round closed without a merge does not
/// make it done. The next part is the one whose round is open, or else the first that is ready: it is the one that
/// waits for an agent of a tier when the item does, and otherwise it is being worked on or ready. Another part is
/// ready when every part it depends on is done, and otherwise waits for those that are not.
fn states_of(plan: &SplitPlan, team: &TeamSummary) -> PartStates {
    let merged = merged_parts(&team.rounds, plan.parent());
    let open = team
        .rounds
        .iter()
        .filter(|round| round.item() == Some(plan.parent()))
        .find(|round| matches!(round.life(), RoundLife::Open))
        .and_then(Round::part);
    let tier = team.tier_waits.get(&plan.parent()).copied();
    let ready = |definition: &PartDefinition| definition.depends_on.iter().all(|key| merged.contains(key));
    let next = open.or_else(|| {
        plan.entries()
            .find(|(id, definition)| !merged.contains(&id.key()) && ready(definition))
            .map(|(id, _)| id.key())
    });
    plan.entries()
        .map(|(id, definition)| {
            let key = id.key();
            let state = if merged.contains(&key) {
                PartState::Done
            } else if next == Some(key) {
                match tier {
                    Some(tier) => PartState::WaitingForTier { tier },
                    None if open == Some(key) => PartState::InProgress,
                    None => PartState::Ready,
                }
            } else {
                let waiting: Vec<PartId> = plan
                    .entries()
                    .filter(|(other, _)| definition.depends_on.contains(&other.key()) && !merged.contains(&other.key()))
                    .map(|(other, _)| other)
                    .collect();
                if waiting.is_empty() {
                    PartState::Ready
                } else {
                    PartState::WaitingFor(waiting)
                }
            };
            (id, state)
        })
        .collect()
}

/// The splits of `team` that `view` prints: every one in the full view, and in the running view those with a part
/// that is not done, since an item whose every part merged is closed and its work is over.
pub(super) fn shown(team: &TeamSummary, view: View) -> Vec<(&SplitPlan, PartStates)> {
    states(team)
        .into_iter()
        .filter(|(_, parts)| view == View::Full || parts.iter().any(|(_, state)| *state != PartState::Done))
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
    for (plan, parts) in shown(team, view) {
        for ((id, definition), (_, state)) in plan.entries().zip(parts) {
            let text = match &state {
                PartState::Done => "done".to_owned(),
                PartState::InProgress => "in progress".to_owned(),
                PartState::Ready => "to do".to_owned(),
                PartState::WaitingForTier { tier } => format!("waiting for {tier} agent"),
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
