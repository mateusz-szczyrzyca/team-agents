//! Tier eligibility and availability for dependency-ready items and split parts.

use super::tier::{TierChoice, choose_agent};
use crate::ids::{AgentName, ItemId};
use crate::kinds::Role;
use crate::kinds::Tier;
use crate::state::{ItemPhase, Member, Participant, Team};
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
    dispatch_in_mode(team, crate::decide::replacement::ReplacementMode::Auto)
}

/// Chooses available participants under a specified replacement mode.
#[must_use]
pub fn dispatch_in_mode(team: &Team, mode: crate::decide::replacement::ReplacementMode) -> Dispatch {
    dispatch_in_policy(team, mode, &[])
}

/// Chooses available participants under a specified replacement policy and configured replacement ordering.
#[must_use]
pub fn dispatch_in_policy(
    team: &Team,
    mode: crate::decide::replacement::ReplacementMode,
    configured_replacements: &[AgentName],
) -> Dispatch {
    dispatch_with_candidates(team, mode, configured_replacements, &[])
}

/// Chooses split replacements using configured tiers and availability supplied by the caller.
#[must_use]
pub fn dispatch_with_candidates(
    team: &Team,
    mode: crate::decide::replacement::ReplacementMode,
    configured_replacements: &[AgentName],
    outside: &[crate::decide::part::Candidate<'_>],
) -> Dispatch {
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
            let ctx = SplitDispatchContext {
                team,
                reserved: &reserved,
                members: &members,
                mode,
                configured_replacements,
                outside_participants: outside,
            };
            match dispatch_split_item(&ctx, item, required) {
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

/// Immutable assignment facts shared by the ready split parts.
struct SplitDispatchContext<'a> {
    /// Team whose parts are ready.
    team: &'a Team,
    /// Participants already assigned during this decision.
    reserved: &'a std::collections::HashSet<String>,
    /// Members eligible for a replacement assignment.
    members: &'a [&'a Member],
    /// Configured handover policy.
    mode: crate::decide::replacement::ReplacementMode,
    /// Replacement names in policy order.
    configured_replacements: &'a [AgentName],
    /// Configured candidates outside the current crew.
    outside_participants: &'a [crate::decide::part::Candidate<'a>],
}

fn dispatch_split_item<'a>(
    ctx: &SplitDispatchContext<'a>,
    item: ItemId,
    required: Tier,
) -> Result<Participant, Waiting> {
    let busy_on_other = |agent_name: &str| {
        ctx.team.items().iter().any(|other| {
            other.id() != item
                && match other.phase() {
                    ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
                        assignee.as_str() == agent_name
                    }
                    _ => false,
                }
        }) || ctx.reserved.contains(agent_name)
    };
    let candidate_of = |member: &'a Member| crate::decide::part::Candidate {
        agent: member.agent(),
        status: member.status(),
        handed_over: member.owes_lead(),
        busy: busy_on_other(member.agent().name()),
    };
    let prev_round = ctx.team.rounds().iter().rev().find(|r| r.item() == Some(item));
    let prev_task = prev_round.and_then(|round| {
        ctx.team
            .members_in(round.id())
            .into_iter()
            .find(|(role, _)| matches!(role, Role::Solo | Role::Member))
            .or_else(|| {
                ctx.team
                    .members_in(round.id())
                    .into_iter()
                    .find(|(role, _)| matches!(role, Role::Coordinator))
            })
            .map(|(_, m)| m.task().clone())
    });
    let prev_member = prev_task.as_ref().and_then(|task| {
        ctx.team
            .crew()
            .roster()
            .iter()
            .find(|(_, m)| m.task() == task)
            .map(|(_, m)| *m)
    });
    let is_coord = |m: &Member| ctx.team.crew().coordinator().is_some_and(|c| c.task() == m.task());
    let (previous, replacements, lead): (
        Option<crate::decide::part::Candidate<'_>>,
        Vec<crate::decide::part::Candidate<'_>>,
        Option<crate::decide::part::Candidate<'_>>,
    ) = {
        let mut reps = Vec::new();
        if !ctx.configured_replacements.is_empty() {
            for rep_name in ctx.configured_replacements {
                if let Some(m) = ctx.members.iter().find(|m| m.agent().name() == rep_name.as_str()) {
                    if prev_member.is_none_or(|p| p.task() != m.task()) {
                        reps.push(candidate_of(m));
                    }
                } else if let Some(p) = ctx
                    .outside_participants
                    .iter()
                    .find(|p| p.agent.name() == rep_name.as_str())
                {
                    reps.push(crate::decide::part::Candidate {
                        agent: p.agent,
                        status: p.status,
                        handed_over: p.handed_over,
                        busy: p.busy || ctx.reserved.contains(p.agent.name()),
                    });
                }
            }
        }

        let prev_cand = prev_member.map(candidate_of);
        let lead_cand = if prev_member.is_some_and(is_coord) {
            None
        } else {
            ctx.team.crew().coordinator().map(candidate_of)
        };

        (prev_cand, reps, lead_cand)
    };

    match crate::decide::part::who_gets_part_in_mode(required, previous, &replacements, lead, ctx.mode) {
        crate::decide::part::PartChoice::PreviousMember(agent)
        | crate::decide::part::PartChoice::Replacement(agent)
        | crate::decide::part::PartChoice::Lead(agent) => Ok(agent.clone()),
        crate::decide::part::PartChoice::Nobody => {
            let fitting = |candidate: crate::decide::part::Candidate<'_>, tier| {
                !matches!(
                    candidate.status,
                    TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done
                ) && !candidate.handed_over
                    && candidate.agent.tier() >= tier
            };
            let previous_fits = previous.is_some_and(|candidate| fitting(candidate, required));
            let fitting = previous_fits
                || match ctx.mode {
                    crate::decide::replacement::ReplacementMode::Off => false,
                    crate::decide::replacement::ReplacementMode::Lead => {
                        lead.is_some_and(|candidate| fitting(candidate, required))
                    }
                    crate::decide::replacement::ReplacementMode::Auto => {
                        let tier = previous.map_or(required, |candidate| candidate.agent.tier().max(required));
                        replacements.iter().any(|candidate| fitting(*candidate, tier))
                            || lead.is_some_and(|candidate| fitting(candidate, required))
                    }
                };
            Err(Waiting {
                item,
                required,
                reason: if fitting { WaitReason::Busy } else { WaitReason::Absent },
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
