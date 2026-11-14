//! The owner's side of rounds: a round is opened when its item is first given to a member (or when a team starts on a
//! list without items), closed when its item is merged or approved, or abandoned when its team is stopped or ends
//! without it; and every run names the rounds it works for from the moment it starts.
//!
//! What is decided is in `agent_core::decide::round::rounds_of_run` and in the checks of the state; this records it, in
//! the state and in the events, so that a daemon that restarts goes on with the same rounds.

use agent_core::decide::round::{RunStart, TaskRounds, rounds_of_run};
use agent_core::ids::{ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::protocol::EventKind;
use agent_core::state::{ItemPhase, Participant, ReviewOutcome, RoundEnd, State, Team, Timestamp};
use agent_core::status::TaskStatus;

use super::start::assigned_item;
use super::teams::Look;
use super::{Owner, OwnerError};
use crate::daemon::team::Why;

/// The rounds the run of `task` is to work for when it starts because of `why` (none: the plain continuation of the
/// task), as the state says.
///
/// # Errors
/// [`OwnerError::LostTask`] when the task is not in the state.
pub(super) fn rounds_for(state: &State, task: &TaskId, why: Option<&Why>) -> Result<Vec<RoundId>, OwnerError> {
    let found = state
        .find_task(task)
        .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
    let (team, role, member) = (found.team, found.role, found.member);
    let previous: Vec<RoundId> = member
        .runs()
        .last()
        .map(|run| run.rounds().to_vec())
        .unwrap_or_default();
    let asked_by: Vec<RoundId> = match why {
        Some(Why::Delegated(request)) => requester_rounds(team, &request.from_task),
        _ => Vec::new(),
    };
    let reviewed = reviewed_round(team, task);
    let goes_on = !member.runs().is_empty();
    let start = match why {
        Some(Why::Delegated(_)) => RunStart::Request { asked_by: &asked_by },
        Some(Why::Review | Why::VerdictMissing) => RunStart::Review {
            round: reviewed.as_ref(),
        },
        // A reviewer's task is made when its review is asked: its first run is that review, with no reason given.
        None if role == Role::Reviewer && !goes_on => RunStart::Review {
            round: reviewed.as_ref(),
        },
        Some(Why::FixPass | Why::AskHuman | Why::Checks) | None if goes_on => {
            RunStart::Continuation { previous: &previous }
        }
        Some(Why::Woken | Why::Assigned(_) | Why::FixPass | Why::AskHuman | Why::Checks) | None => RunStart::Work,
    };
    let item = member_item_round(team, role, member.agent());
    let open: Vec<RoundId> = team.open_rounds().map(|round| round.id().clone()).collect();
    Ok(rounds_of_run(
        role,
        start,
        &TaskRounds {
            item: item.as_ref(),
            open: &open,
        },
    ))
}

/// The rounds the latest run of `requester`, a task of `team`, works for: the rounds a worker asked by it works for.
fn requester_rounds(team: &Team, requester: &TaskId) -> Vec<RoundId> {
    team.find(requester)
        .and_then(|(_, member)| member.runs().last().map(|run| run.rounds().to_vec()))
        .unwrap_or_default()
}

/// The round of the pull request that the reviewer task `task` has a review pending of.
fn reviewed_round(team: &Team, task: &TaskId) -> Option<RoundId> {
    team.items().iter().find_map(|progress| {
        let ItemPhase::InReview { reviews, .. } = progress.phase() else {
            return None;
        };
        reviews
            .iter()
            .any(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending))
            .then(|| team.round_of(progress.id()).map(|round| round.id().clone()))
            .flatten()
    })
}

/// The round of the item that the participant `participant`, who has `role` in `team`, works on, if it has one. A solo
/// agent on a list without items does the whole work, which is the one round the team has.
fn member_item_round(team: &Team, role: Role, participant: &Participant) -> Option<RoundId> {
    if role == Role::Solo && team.items().is_empty() {
        return team.rounds().first().map(|round| round.id().clone());
    }
    let item = assigned_item(team, participant)?;
    team.round_of(item.id).map(|round| round.id().clone())
}

impl Owner {
    /// A round ID that no round of the state has.
    fn fresh_round_id(&mut self) -> Result<RoundId, OwnerError> {
        for _ in 0..ID_ATTEMPTS {
            let id = RoundId::generate(&self.parts.clock, &mut self.parts.rng)?;
            if self.parts.state.find_round(&id).is_none() {
                return Ok(id);
            }
        }
        Err(OwnerError::NoFreeRoundId)
    }

    /// Opens the round of `item` of `team` when the item has none — it is given to a member for the first time — and says
    /// so in `look`. An item that has a round already, closed or not, keeps it: an item has one.
    ///
    /// # Errors
    /// [`OwnerError`] when the state refuses the round.
    pub(super) fn open_item_round(
        &mut self,
        team: &TeamId,
        item: ItemId,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let has_round = self
            .parts
            .state
            .team(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?
            .round_of(item)
            .is_some();
        if has_round {
            return Ok(());
        }
        self.open_round(team, Some(item), now, look)
    }

    /// Opens a round of `team` for `item` — none: the whole work of a team whose list has no items.
    pub(super) fn open_round(
        &mut self,
        team: &TeamId,
        item: Option<ItemId>,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let round = self.fresh_round_id()?;
        self.parts
            .state
            .open_round(team, round.clone(), item, now)
            .map_err(|source| OwnerError::OpenRound {
                team: team.clone(),
                source,
            })?;
        self.record_round(team, &round);
        look.events.push(EventKind::RoundOpened {
            team: team.clone(),
            round: round.clone(),
            item,
        });
        look.changed = true;
        tracing::info!(team = %team, round = %round, ?item, "a round is opened");
        Ok(())
    }

    /// The round of `item` of `team`.
    ///
    /// # Errors
    /// [`OwnerError::LostRound`] when the item has none.
    pub(super) fn round_of_item(&self, team: &TeamId, item: ItemId) -> Result<RoundId, OwnerError> {
        self.parts
            .state
            .team(team)
            .and_then(|found| found.round_of(item))
            .map(|round| round.id().clone())
            .ok_or_else(|| OwnerError::LostRound {
                team: team.clone(),
                item,
            })
    }

    /// Closes the round of `item` of `team` as `how` when it is open, and says so in `look`. A round that is closed
    /// already — abandoned when its team was stopped, then resumed — stays as it is.
    ///
    /// # Errors
    /// [`OwnerError`] when the state refuses the ending.
    pub(super) fn close_item_round(
        &mut self,
        team: &TeamId,
        item: ItemId,
        how: RoundEnd,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let open = self
            .parts
            .state
            .team(team)
            .and_then(|found| found.round_of(item))
            .filter(|round| round.is_open())
            .map(|round| round.id().clone());
        match open {
            Some(round) => self.close_round(team, &round, Some(item), how, now, look),
            None => Ok(()),
        }
    }

    /// Closes the round `round` of `team` — of `item`, if it has one — as `how` and says so in `look`.
    fn close_round(
        &mut self,
        team: &TeamId,
        round: &RoundId,
        item: Option<ItemId>,
        how: RoundEnd,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        self.parts
            .state
            .close_round(round, now, how)
            .map_err(|source| OwnerError::CloseRound {
                round: round.clone(),
                source,
            })?;
        look.events.push(EventKind::RoundClosed {
            team: team.clone(),
            round: round.clone(),
            item,
            how,
        });
        look.changed = true;
        tracing::info!(team = %team, round = %round, ?how, "a round is closed");
        Ok(())
    }

    /// Abandons every round of `team` that is open: its team was stopped or ended without them.
    ///
    /// # Errors
    /// [`OwnerError`] when the state refuses an ending.
    pub(super) fn abandon_open_rounds(
        &mut self,
        team: &TeamId,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let open = open_rounds_of(&self.parts.state, team)?;
        for (round, item) in open {
            self.close_round(team, &round, item, RoundEnd::Abandoned, now, look)?;
        }
        Ok(())
    }
}

/// The rounds of `team` that are open, oldest first, each with its item (none for the whole work of a team whose list has
/// no items).
///
/// # Errors
/// [`OwnerError::LostTeam`] when the team is not in the state.
fn open_rounds_of(state: &State, team: &TeamId) -> Result<Vec<(RoundId, Option<ItemId>)>, OwnerError> {
    let found = state
        .team(team)
        .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
    Ok(found
        .open_rounds()
        .map(|round| (round.id().clone(), round.item()))
        .collect())
}

/// How many round IDs are drawn before the owner gives up: two IDs of one second differ in four characters drawn at
/// random, so a second draw is all but certain to be free.
const ID_ATTEMPTS: usize = 8;

/// Whether every task of the crew of `team` is stopped, failed or done: the team has nobody left to work, which is what a
/// stop of the human leaves it.
pub(super) fn is_stopped(team: &Team) -> bool {
    team.crew().roster().into_iter().all(|(_, member)| {
        matches!(
            member.status(),
            TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done
        )
    })
}

#[cfg(test)]
mod tests;
