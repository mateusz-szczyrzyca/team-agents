//! Tier eligibility and availability for dependency-ready items and split parts.

use super::tier::{TierChoice, choose_agent};
use crate::ids::ItemId;
use crate::kinds::Role;
use crate::kinds::Tier;
use crate::state::{Member, Participant, Team};
use crate::status::TaskStatus;

/// Why a ready item cannot start yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// A qualified participant belongs to the team but is unavailable.
    Busy,
    /// No participant in the crew has the required capability.
    Absent,
}

/// One ready item whose capability requirement cannot currently be met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waiting {
    /// The original item, also for a split part.
    pub item: ItemId,
    /// The minimum capability.
    pub required: Tier,
    /// Whether capability is absent or merely unavailable.
    pub reason: WaitReason,
}

/// Assignments and explicit waits from one immutable team snapshot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dispatch {
    /// Ready items paired with their selected participant.
    pub assigned: Vec<(ItemId, Participant)>,
    /// Ready items which remain pending.
    pub waiting: Vec<Waiting>,
}

/// Chooses available participants without lowering an item's or part's tier.
#[must_use]
pub fn dispatch(team: &Team) -> Dispatch {
    let sets = super::ItemSets::of(team.items());
    let features = team.task_list().list().features();
    let ready = features
        .iter()
        .filter(|feature| super::dependencies_merged(feature, features, &sets))
        .filter_map(|feature| super::next_item(feature, &sets, team.merge()));
    let roster = team.crew().roster();
    let members: Vec<_> = roster
        .iter()
        .filter(|(role, _)| matches!(role, Role::Solo | Role::Member))
        .map(|(_, member)| *member)
        .collect();
    let mut reserved = std::collections::HashSet::new();
    let mut result = Dispatch::default();
    for item in ready {
        let required = required_tier(team, item);
        let fitting_member = members.iter().any(|member| member.agent().tier() >= required);
        let coordinator = team.crew().coordinator().filter(|_| !fitting_member);
        let available = |member: &Member| {
            matches!(member.status(), TaskStatus::Queued | TaskStatus::Waiting)
                && !sets.busy.iter().any(|name| name.as_str() == member.agent().name())
                && !reserved.contains(member.agent().name())
        };
        let offered: Vec<_> = members
            .iter()
            .filter(|member| available(member))
            .map(|member| member.agent().clone())
            .collect();
        match choose_agent(
            Some(required),
            &offered,
            coordinator.filter(|member| available(member)).map(Member::agent),
        ) {
            TierChoice::Member(agent) | TierChoice::Coordinator(agent) => {
                reserved.insert(agent.name().to_owned());
                result.assigned.push((item, agent.clone()));
            }
            TierChoice::NoAgent { required } => result.waiting.push(Waiting {
                item,
                required,
                reason: if fitting_member || coordinator.is_some_and(|member| member.agent().tier() >= required) {
                    WaitReason::Busy
                } else {
                    WaitReason::Absent
                },
            }),
        }
    }
    result
}

/// The item's minimum capability, defaulting to normal for an omitted directive.
#[must_use]
pub fn required_tier(team: &Team, item: ItemId) -> Tier {
    if let Some((part, _)) = team.next_part(item) {
        return team
            .splits()
            .iter()
            .find(|plan| plan.parent() == item)
            .and_then(|plan| plan.requirements(part.key()))
            .map_or(Tier::Normal, |requirements| requirements.tier);
    }
    team.task_list().list().required_tier(item).unwrap_or(Tier::Normal)
}

#[cfg(test)]
mod tests;

/// Working items whose assignee cannot deliver their current part's tier.
#[must_use]
pub fn underqualified(team: &Team) -> Vec<ItemId> {
    team.items()
        .iter()
        .filter_map(|item| {
            let crate::state::ItemPhase::InProgress { assignee, .. } = item.phase() else {
                return None;
            };
            let required = required_tier(team, item.id());
            team.crew()
                .roster()
                .iter()
                .find(|(_, member)| member.agent().name() == assignee.as_str())
                .filter(|(_, member)| member.agent().tier() < required)
                .map(|_| item.id())
        })
        .collect()
}

/// Whether the coordinator meets every capability required by a member's working items.
#[must_use]
pub fn coordinator_can_take_over(team: &Team, member: &Member) -> bool {
    team.crew().coordinator().is_some_and(|coordinator| {
        team.items().iter().all(|item| match item.phase() {
            crate::state::ItemPhase::InProgress { assignee, .. } | crate::state::ItemPhase::Fixing { assignee, .. } => {
                assignee.as_str() != member.agent().name()
                    || coordinator.agent().tier() >= required_tier(team, item.id())
            }
            crate::state::ItemPhase::Pending
            | crate::state::ItemPhase::InReview { .. }
            | crate::state::ItemPhase::Approved { .. }
            | crate::state::ItemPhase::Done { .. } => true,
        })
    })
}
