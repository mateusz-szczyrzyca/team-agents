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
        let available = |member: &Member| {
            matches!(member.status(), TaskStatus::Queued | TaskStatus::Waiting)
                && !sets.busy.iter().any(|name| name.as_str() == member.agent().name())
                && !reserved.contains(member.agent().name())
        };

        if team.next_part(item).is_some() {
            match dispatch_split_item(team, item, required, &sets, &reserved, &members) {
                Ok(agent) => {
                    reserved.insert(agent.name().to_owned());
                    result.assigned.push((item, agent));
                }
                Err(waiting) => {
                    result.waiting.push(waiting);
                }
            }
            continue;
        }

        let fitting_member = members.iter().any(|member| {
            !matches!(member.status(), TaskStatus::Stopped | TaskStatus::Failed)
                && !member.owes_lead()
                && member.agent().tier() >= required
        });
        let coordinator = team.crew().coordinator().filter(|_| !fitting_member);
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

fn dispatch_split_item<'a>(
    team: &'a Team,
    item: ItemId,
    required: Tier,
    sets: &super::ItemSets<'_>,
    reserved: &std::collections::HashSet<String>,
    members: &[&'a Member],
) -> Result<Participant, Waiting> {
    let candidate_of = |member: &'a Member| crate::decide::part::Candidate {
        agent: member.agent(),
        status: member.status(),
        handed_over: !member.past_holders().is_empty() || member.owes_lead(),
        busy: sets.busy.iter().any(|name| name.as_str() == member.agent().name())
            || reserved.contains(member.agent().name()),
    };
    let prev_round = team.rounds().iter().rev().find(|r| r.item() == Some(item));
    let prev_agent_name = prev_round.and_then(|round| {
        team.part_assignments().get(round.id()).cloned().or_else(|| {
            team.members_in(round.id())
                .into_iter()
                .find(|(role, _)| matches!(role, Role::Solo | Role::Member | Role::Coordinator))
                .map(|(_, m)| m.agent().agent_name())
        })
    });
    let prev_member = prev_agent_name.as_ref().and_then(|name| {
        team.crew()
            .roster()
            .iter()
            .find(|(_, m)| m.agent().name() == name.as_str())
            .map(|(_, m)| *m)
    });
    let is_coord = |m: &Member| team.crew().coordinator().is_some_and(|c| c.task() == m.task());
    let (previous, replacements, lead): (
        Option<crate::decide::part::Candidate<'_>>,
        Vec<crate::decide::part::Candidate<'_>>,
        Option<crate::decide::part::Candidate<'_>>,
    ) = if let Some(prev) = prev_member {
        if is_coord(prev) {
            (
                Some(candidate_of(prev)),
                members.iter().map(|m| candidate_of(m)).collect(),
                None,
            )
        } else {
            let reps = members
                .iter()
                .filter(|m| m.task() != prev.task())
                .map(|m| candidate_of(m))
                .collect();
            let lead = team.crew().coordinator().map(candidate_of);
            (Some(candidate_of(prev)), reps, lead)
        }
    } else {
        let reps = members.iter().map(|m| candidate_of(m)).collect();
        let lead = team.crew().coordinator().map(candidate_of);
        (None, reps, lead)
    };

    match crate::decide::part::who_gets_part(required, previous, &replacements, lead) {
        crate::decide::part::PartChoice::PreviousMember(agent)
        | crate::decide::part::PartChoice::Replacement(agent)
        | crate::decide::part::PartChoice::Lead(agent) => Ok(agent.clone()),
        crate::decide::part::PartChoice::Nobody => {
            let fitting_in_crew = team.crew().roster().iter().any(|(_, member)| {
                !matches!(member.status(), TaskStatus::Stopped | TaskStatus::Failed)
                    && !member.owes_lead()
                    && member.agent().tier() >= required
            });
            Err(Waiting {
                item,
                required,
                reason: if fitting_in_crew {
                    WaitReason::Busy
                } else {
                    WaitReason::Absent
                },
            })
        }
    }
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

/// The qualified holder of a review fix, preserving a fitting integration lead.
#[must_use]
pub fn fixer(team: &Team, item: ItemId) -> Option<&Member> {
    let required = required_tier(team, item);
    let roster = team.crew().roster();
    let (_, lead) = roster.first()?;
    if lead.agent().tier() >= required {
        return Some(lead);
    }
    roster
        .iter()
        .find(|(role, member)| matches!(role, Role::Member) && member.agent().tier() >= required)
        .map(|(_, member)| *member)
}
