//! Shared split provenance displayed by the CLI and published on pull requests.

use agent_core::kinds::Tier;
use agent_core::state::Round;
use agent_core::state::split::{PartKey, SplitApproval, SplitPlan};

/// Describes each part from durable metadata; absent legacy facts remain explicitly unknown.
pub(crate) fn lines(
    plan: &SplitPlan,
    rounds: &[Round],
    assignments: &std::collections::BTreeMap<agent_core::ids::RoundId, agent_core::ids::AgentName>,
) -> Vec<(PartKey, String)> {
    let approval = match plan.approved_by() {
        Some(SplitApproval::Harness) => "harness",
        Some(SplitApproval::Human) => "human",
        None => "not recorded",
    };
    plan.entries().map(|(id, _)| {
        let round = rounds.iter().rev().find(|round| round.item() == Some(plan.parent()) && round.part() == Some(id.key()));
        let agent = round.map_or("not assigned", |round| assignments.get(round.id()).map_or("not recorded", |agent| agent.as_str()));
        // Legacy parts omit requirements and request the normal tier.
        let tier = plan.requirements(id.key()).map_or(Tier::Normal, |requirements| requirements.tier);
        let siblings = plan.entries().filter(|(other, _)| other.key() != id.key()).map(|(other, _)| {
            let pr = rounds.iter().rev().find(|round| round.item() == Some(plan.parent()) && round.part() == Some(other.key()))
                .and_then(Round::part_pull_request);
            pr.map_or_else(|| format!("{other} (not published)"), |pr| format!("{other} (#{})", pr.number))
        }).collect::<Vec<_>>().join(", ");
        (id.key(), format!("Split {id} (item {}, part {}); approved by {approval}; requested {tier}; assigned {agent}; other parts: {siblings}.", plan.parent(), id.key()))
    }).collect()
}
