//! One activity order for all investigator requests, with permanent and last-request exclusions.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use rand::Rng;

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
/// Crew and caller exclusions are permanent. Candidates are ordered by their last
/// activity for the team, then by their last activity anywhere, then by name.
/// Candidates tied on the oldest last activity are drawn at random with `rng`; no
/// activity sorts before any timestamp. The investigators of the latest request for the
/// team have the newest activity in it, so they come after every other candidate and
/// are chosen only when too few others are left (the relaxation of the latest request).
#[must_use]
pub fn choose(
    configured: &BTreeSet<InvestigatorName>,
    free: &BTreeSet<InvestigatorName>,
    activities: &[Activity<'_>],
    team: &TeamId,
    excluded: &BTreeSet<InvestigatorName>,
    required: NonZeroUsize,
    rng: &mut impl Rng,
) -> Option<Vec<InvestigatorName>> {
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
        candidates.push(((last_team, last_anywhere), name));
    }
    if candidates.len() < required.get() {
        return None;
    }
    candidates.sort_unstable();

    let req = required.get();
    let boundary_key = candidates.get(req.saturating_sub(1)).map(|(k, _)| *k)?;
    let oldest_activity_key = candidates.iter().map(|(k, _)| *k).min();
    // Tie-breaking applies when multiple candidates share the same oldest last activity.
    if Some(boundary_key) == oldest_activity_key {
        let start = candidates.iter().position(|(k, _)| *k == boundary_key).unwrap_or(0);
        let end = candidates
            .iter()
            .rposition(|(k, _)| *k == boundary_key)
            .map_or(candidates.len(), |p| p + 1);
        let candidates_for_key = end - start;
        let slots_for_key = req - start;
        // Shuffle the tied group whenever fewer slots remain than candidates
        // for the tie key — a group of tied candidates larger than the slots
        // left is a tie too. When every tied candidate fits no shuffle is needed.
        if slots_for_key < candidates_for_key {
            use rand::seq::SliceRandom;
            if let Some(tied) = candidates.get_mut(start..end) {
                tied.shuffle(rng);
                if let Some(chosen) = tied.get_mut(..slots_for_key) {
                    chosen.sort_unstable();
                }
            }
        }
    }

    Some(
        candidates
            .into_iter()
            .take(req)
            .map(|(_, name)| (*name).clone())
            .collect(),
    )
}

/// The investigators of `config` that may take a new investigation request: configured,
/// not occupied anywhere, with no task currently [`crate::status::TaskStatus::Paused`], and not refused at `now`.
#[must_use]
pub fn free_for_request(
    state: &crate::state::State,
    config: &crate::config::Config,
    now: Timestamp,
) -> BTreeSet<InvestigatorName> {
    let occupied = occupied_investigators(state);
    let refused = refused_investigators(state, config, now);
    config
        .investigators()
        .keys()
        .filter(|name| {
            !occupied.contains(*name)
                && !refused.contains(name)
                && !has_paused_task(state, name)
                && !provider_cooling_down(state, config, name, now)
        })
        .cloned()
        .collect()
}

/// The newest run controls provider cooldown, regardless of which task used the investigator.
fn provider_cooling_down(
    state: &crate::state::State,
    config: &crate::config::Config,
    name: &InvestigatorName,
    now: Timestamp,
) -> bool {
    use crate::state::{RunEnd, RunState};
    let newest = state
        .teams()
        .iter()
        .flat_map(|team| {
            team.crew()
                .roster()
                .into_iter()
                .map(|(_, member)| member)
                .chain(team.reviewers())
        })
        .filter(|member| investigator_of(member) == Some(name))
        .flat_map(crate::state::Member::runs)
        .max_by_key(|run| run.started_at());
    let Some(run) = newest else {
        return false;
    };
    let RunState::Ended { at, end } = run.state() else {
        return false;
    };
    let elapsed = (now.as_utc() - at.as_utc()).whole_seconds();
    let elapsed = u128::from(elapsed.max(0).unsigned_abs());
    match end {
        RunEnd::QuotaExhausted { resets_at: Some(reset) } => *reset > now,
        RunEnd::QuotaExhausted { resets_at: None } => {
            elapsed < u128::from(config.limits().quota_pause_minutes.as_duration().as_secs())
        }
        RunEnd::RateLimited => elapsed < u128::from(config.limits().rate_limit_pause_minutes.as_duration().as_secs()),
        RunEnd::Reported { .. } | RunEnd::NoStatus { .. } | RunEnd::Hung | RunEnd::TurnLimit | RunEnd::Stopped => false,
    }
}

/// The investigators of `config` that their provider refused at `now`, and those whose account is paused or held at
/// `now`: they review nothing until their return or the reset of their account.
#[must_use]
pub fn refused_investigators(
    state: &crate::state::State,
    config: &crate::config::Config,
    now: Timestamp,
) -> std::collections::HashSet<InvestigatorName> {
    let refused = state.refused_agents_at(now);
    let paused = crate::decide::account_candidates::investigators_of_paused_accounts(state, config, now);
    let held = crate::decide::account_hold::investigators_of_held_accounts(state, config, now);
    config
        .investigators()
        .keys()
        .filter(|name| {
            paused.contains(*name)
                || held.contains(*name)
                || refused.iter().any(|agent| agent.as_str() == name.as_str())
        })
        .cloned()
        .collect()
}

/// Investigators with a running or queued run, including accepted review requests
/// waiting for the scheduler to queue their reusable task.
#[must_use]
pub fn occupied_investigators(state: &crate::state::State) -> BTreeSet<InvestigatorName> {
    state
        .teams()
        .iter()
        .flat_map(|team| {
            team.crew()
                .roster()
                .into_iter()
                .map(|(_, member)| member)
                .chain(team.reviewers().iter())
                .filter(|member| {
                    member.current_run().is_some()
                        || matches!(
                            member.status(),
                            crate::status::TaskStatus::Queued | crate::status::TaskStatus::Running
                        )
                        || (member.status() == crate::status::TaskStatus::Waiting && has_pending_review(team, member))
                })
                .filter_map(investigator_of)
                .cloned()
        })
        .collect()
}

/// The investigator that works in the task of `member`, if it is one.
#[must_use]
pub fn investigator_of(member: &crate::state::Member) -> Option<&InvestigatorName> {
    match member.agent() {
        crate::state::Participant::Investigator(name) => Some(name),
        crate::state::Participant::Normal(_) | crate::state::Participant::Worker(_) => None,
    }
}

/// Whether the reviewer `member` has a review to give for an item of `team` that is in review.
#[must_use]
fn has_pending_review(team: &crate::state::Team, member: &crate::state::Member) -> bool {
    if pending_opinion(team, member.task()).is_some() {
        return true;
    }
    team.items().iter().any(|item| match item.phase() {
        crate::state::ItemPhase::InReview { reviews, .. } => reviews.iter().any(|review| {
            review.task == *member.task() && matches!(review.outcome, crate::state::ReviewOutcome::Pending)
        }),
        crate::state::ItemPhase::Pending
        | crate::state::ItemPhase::InProgress { .. }
        | crate::state::ItemPhase::Fixing { .. }
        | crate::state::ItemPhase::Approved { .. }
        | crate::state::ItemPhase::Done { .. } => false,
    })
}

/// The pending second opinion owned by a task, while its round is being fixed.
#[must_use]
pub fn pending_opinion<'a>(
    team: &'a crate::state::Team,
    task: &crate::ids::TaskId,
) -> Option<(
    crate::ids::ItemId,
    std::num::NonZeroU32,
    &'a crate::state::SecondOpinion,
)> {
    team.items()
        .iter()
        .filter(|item| matches!(item.phase(), crate::state::ItemPhase::Fixing { .. }))
        .find_map(|item| {
            item.findings()?.opinions().iter().find_map(|(number, opinion)| {
                matches!(&opinion.stage, crate::state::OpinionStage::Assigned { task: owner, .. } if owner == task)
                    .then_some((item.id(), *number, opinion))
            })
        })
}

fn has_paused_task(state: &crate::state::State, name: &InvestigatorName) -> bool {
    state.teams().iter().any(|team| {
        team.crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers().iter())
            .any(|member| investigator_of(member) == Some(name) && member.status() == crate::status::TaskStatus::Paused)
    })
}

#[cfg(test)]
mod tests;
