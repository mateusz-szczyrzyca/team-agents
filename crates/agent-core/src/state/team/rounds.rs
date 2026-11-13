//! The rounds of a team: opening and closing them, and the checks that tie them to the team's items and runs.
//!
//! The operations check as they go, and [`check_rounds`] checks the same when a state is read, so that a state that is
//! read is as valid as one that was built through the operations. What a round has to do with the other parts:
//!
//! - an item has at most one round, and has one as soon as it leaves `pending` (an item the list marks `[x]` has none,
//!   as it never starts);
//! - a closed round is never opened again; one closed as `merged` belongs to an item done as merged, and one closed as
//!   `approved` to an item that is approved or done as merged;
//! - a round without an item is the whole work of a team whose list has no items, and there is one at most;
//! - a run names only rounds of its own team, none twice.

use std::collections::HashSet;

use super::{Team, TeamError};
use crate::ids::{ItemId, RoundId, RunId, TaskId};
use crate::kinds::Role;
use crate::state::item::{AdvanceError, Completion, ItemPhase, ItemProgress, PhaseKind};
use crate::state::member::Member;
use crate::state::round::{ClosedBeforeOpened, Round, RoundEnd};
use crate::state::run::RunError;
use crate::state::values::{SessionId, Timestamp};

/// What a run that starts is given, besides the task it starts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    /// The run's ID.
    pub id: RunId,
    /// When it starts.
    pub at: Timestamp,
    /// The CLI's session, if it is known already.
    pub session: Option<SessionId>,
    /// The rounds it works for, oldest first.
    pub rounds: Vec<RoundId>,
}

/// Why a round could not be opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenRoundError {
    /// The team has a round with this ID.
    #[error("the team has the round {round} already")]
    DuplicateId {
        /// The repeated ID.
        round: RoundId,
    },
    /// The team has no such item.
    #[error("the team has no item {item}")]
    UnknownItem {
        /// The item.
        item: ItemId,
    },
    /// The item has a round already: an item has one round, which is never opened again.
    #[error("item {item} has the round {round} already")]
    ItemHasRound {
        /// The item.
        item: ItemId,
        /// Its round.
        round: RoundId,
    },
    /// The item is done: there is no work left to be a round.
    #[error("item {item} is done")]
    ItemDone {
        /// The item.
        item: ItemId,
    },
    /// A round without an item is for a task list without items.
    #[error("a round without an item is for a task list without items, and this one has items")]
    NoItemInTeamWithItems,
    /// The team has the round of its whole work already.
    #[error("the team has the round {round} of its whole work already")]
    WholeWorkTwice {
        /// The round of the whole work.
        round: RoundId,
    },
}

/// Why a round could not be closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloseRoundError {
    /// There is no such round.
    #[error("there is no round {round}")]
    NoSuchRound {
        /// The round.
        round: RoundId,
    },
    /// The round is closed already, and a closed round stays as it is.
    #[error("round {round} is closed already")]
    AlreadyClosed {
        /// The round.
        round: RoundId,
    },
    /// The end is before the opening.
    #[error(transparent)]
    BeforeOpening(#[from] ClosedBeforeOpened),
    /// The item of the round is not where this ending needs it.
    #[error("round {round} cannot end as {how}: {}", describe_item(.item))]
    EndDoesNotFit {
        /// The round.
        round: RoundId,
        /// The refused ending.
        how: RoundEnd,
        /// The phase of the round's item; none for a round without an item.
        item: Option<PhaseKind>,
    },
}

/// Why an item could not be moved on by [`Team::advance_item`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdvanceItemError {
    /// The team has no such item.
    #[error("the team has no item {item}")]
    NoSuchItem {
        /// The item.
        item: ItemId,
    },
    /// An item leaves `pending` only with a round.
    #[error("item {item} cannot leave pending: it has no round")]
    NoRound {
        /// The item.
        item: ItemId,
    },
    /// The change of the item's phase is not allowed.
    #[error(transparent)]
    Advance(#[from] AdvanceError),
}

/// Why a run could not be started by [`Team::begin_run`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BeginRunError {
    /// The team has no such task.
    #[error("the team has no task {task}")]
    UnknownTask {
        /// The task.
        task: TaskId,
    },
    /// The run names a round the team does not have.
    #[error("the run names the round {round}, which the team does not have")]
    UnknownRound {
        /// The round.
        round: RoundId,
    },
    /// The run cannot be recorded for the task.
    #[error(transparent)]
    Run(#[from] RunError),
}

/// What is wrong with the rounds of a team that is read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoundsError {
    /// Two rounds have one ID.
    #[error("the round {round} appears twice")]
    DuplicateRound {
        /// The repeated ID.
        round: RoundId,
    },
    /// A round is for an item the task list does not have.
    #[error("round {round} is for item {item}, which the team does not have")]
    UnknownItem {
        /// The round.
        round: RoundId,
        /// The item.
        item: ItemId,
    },
    /// An item has two rounds.
    #[error("item {item} has two rounds, {first} and {second}")]
    ItemWithTwoRounds {
        /// The item.
        item: ItemId,
        /// Its first round.
        first: RoundId,
        /// Its second round.
        second: RoundId,
    },
    /// A round has no item although the task list has items.
    #[error("round {round} has no item, but the task list has items")]
    NoItemInTeamWithItems {
        /// The round.
        round: RoundId,
    },
    /// A task list without items has two rounds of its whole work.
    #[error("the rounds {first} and {second} are both the whole work of the team")]
    WholeWorkTwice {
        /// The first round.
        first: RoundId,
        /// The second round.
        second: RoundId,
    },
    /// An item the list marks `[x]` never starts, so it has no round.
    #[error("round {round} is for item {item}, which the task list marks done")]
    RoundOfListedItem {
        /// The round.
        round: RoundId,
        /// The item.
        item: ItemId,
    },
    /// An item has left `pending` without a round.
    #[error("item {item} is {phase} but has no round")]
    ItemWithoutRound {
        /// The item.
        item: ItemId,
        /// Its phase.
        phase: PhaseKind,
    },
    /// A round ended in a way its item does not fit.
    #[error("round {round} ended as {how}, but {}", describe_item(.item))]
    EndDoesNotFit {
        /// The round.
        round: RoundId,
        /// How it ended.
        how: RoundEnd,
        /// The phase of the round's item; none for a round without an item.
        item: Option<PhaseKind>,
    },
    /// A run names a round the team does not have.
    #[error("run {run} of task {task} names the round {round}, which the team does not have")]
    RunNamesUnknownRound {
        /// The task.
        task: TaskId,
        /// The run.
        run: RunId,
        /// The round.
        round: RoundId,
    },
}

/// Where a round's item is, for a message.
#[expect(
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "thiserror hands the field of the error to the function by reference"
)]
fn describe_item(item: &Option<PhaseKind>) -> String {
    item.map_or_else(|| "it has no item".to_owned(), |phase| format!("its item is {phase}"))
}

impl Team {
    getter!("The rounds of the team, oldest first.", pub fn rounds(&self) -> &[Round] = &self.rounds);

    /// The round `id`.
    #[must_use]
    pub fn round(&self, id: &RoundId) -> Option<&Round> {
        self.rounds.iter().find(|round| round.id() == id)
    }

    /// The round of item `item`; none for an item that has not started.
    #[must_use]
    pub fn round_of(&self, item: ItemId) -> Option<&Round> {
        self.rounds.iter().find(|round| round.item() == Some(item))
    }

    /// The rounds that are open, oldest first.
    pub fn open_rounds(&self) -> impl Iterator<Item = &Round> {
        self.rounds.iter().filter(|round| round.is_open())
    }

    /// The members that worked for round `round` — a run of theirs names it — with their roles, in the order of the
    /// team: the crew, then the reviewers.
    #[must_use]
    pub fn members_in(&self, round: &RoundId) -> Vec<(Role, &Member)> {
        let reviewers = self.reviewers.iter().map(|member| (Role::Reviewer, member));
        self.crew
            .roster()
            .into_iter()
            .chain(reviewers)
            .filter(|(_, member)| member.runs().iter().any(|run| run.rounds().contains(round)))
            .collect()
    }

    /// Opens a round with the ID `id` for `item` — or, for a task list without items, for the whole work of the team —
    /// at `at`.
    ///
    /// # Errors
    /// [`OpenRoundError`]: the team has a round with this ID, the item is not the team's, has a round already or is
    /// done; a round without an item in a team whose list has items, or a second one in a team whose list has none.
    pub fn open_round(&mut self, id: RoundId, item: Option<ItemId>, at: Timestamp) -> Result<(), OpenRoundError> {
        if self.rounds.iter().any(|round| round.id() == &id) {
            return Err(OpenRoundError::DuplicateId { round: id });
        }
        if let Some(item) = item {
            let progress = self.item(item).ok_or(OpenRoundError::UnknownItem { item })?;
            if let Some(existing) = self.round_of(item) {
                return Err(OpenRoundError::ItemHasRound {
                    item,
                    round: existing.id().clone(),
                });
            }
            if matches!(progress.phase(), ItemPhase::Done { .. }) {
                return Err(OpenRoundError::ItemDone { item });
            }
        } else {
            if !self.items.is_empty() {
                return Err(OpenRoundError::NoItemInTeamWithItems);
            }
            if let Some(existing) = self.rounds.first() {
                return Err(OpenRoundError::WholeWorkTwice {
                    round: existing.id().clone(),
                });
            }
        }
        self.rounds.push(Round::opened(id, item, at));
        Ok(())
    }

    /// Closes the round `id` at `at`, ended as `how`.
    ///
    /// # Errors
    /// [`CloseRoundError`]: the team has no such round, it is closed already, `at` is before it was opened, or its item
    /// is not where `how` needs it (`merged` needs it done as merged, `approved` approved or done as merged).
    pub fn close_round(&mut self, id: &RoundId, at: Timestamp, how: RoundEnd) -> Result<(), CloseRoundError> {
        let round = self
            .round(id)
            .ok_or_else(|| CloseRoundError::NoSuchRound { round: id.clone() })?;
        if !round.is_open() {
            return Err(CloseRoundError::AlreadyClosed { round: id.clone() });
        }
        let phase = round.item().and_then(|item| self.item(item)).map(ItemProgress::phase);
        if !end_fits(how, phase) {
            return Err(CloseRoundError::EndDoesNotFit {
                round: id.clone(),
                how,
                item: phase.map(ItemPhase::kind),
            });
        }
        self.rounds
            .iter_mut()
            .find(|round| round.id() == id)
            .ok_or_else(|| CloseRoundError::NoSuchRound { round: id.clone() })?
            .close(at, how)?;
        Ok(())
    }

    /// Moves item `id` to `next`, as [`ItemMut::advance`](crate::state::ItemMut::advance) does, and refuses to take an
    /// item out of `pending` that has no round.
    ///
    /// # Errors
    /// [`AdvanceItemError`]: the team has no such item, it has no round and is leaving `pending`, or its phase may not
    /// change to `next`.
    pub fn advance_item(&mut self, id: ItemId, next: ItemPhase) -> Result<(), AdvanceItemError> {
        let progress = self.item(id).ok_or(AdvanceItemError::NoSuchItem { item: id })?;
        let leaving = matches!(progress.phase(), ItemPhase::Pending) && !matches!(next, ItemPhase::Pending);
        if leaving && self.round_of(id).is_none() {
            return Err(AdvanceItemError::NoRound { item: id });
        }
        let mut item = self.item_mut(id).ok_or(AdvanceItemError::NoSuchItem { item: id })?;
        item.advance(next)?;
        Ok(())
    }

    /// Records that a run of task `task` has started, working for the rounds of `run`.
    ///
    /// # Errors
    /// [`BeginRunError`]: the team has no such task, a round of the run is not the team's, or the run cannot be recorded
    /// ([`RunError`]: one is going, the ID is used, a round is named twice).
    pub fn begin_run(&mut self, task: &TaskId, run: NewRun) -> Result<(), BeginRunError> {
        let NewRun {
            id,
            at,
            session,
            rounds,
        } = run;
        if let Some(unknown) = rounds.iter().find(|named| self.round(named).is_none()) {
            return Err(BeginRunError::UnknownRound { round: unknown.clone() });
        }
        let (_, member) = self
            .find_mut(task)
            .ok_or_else(|| BeginRunError::UnknownTask { task: task.clone() })?;
        member.value.begin_run_for(id, at, session, rounds)?;
        Ok(())
    }
}

/// Whether a round may end as `how` when its item is in `phase` (none: the round has no item).
fn end_fits(how: RoundEnd, phase: Option<&ItemPhase>) -> bool {
    let merged = matches!(
        phase,
        Some(ItemPhase::Done {
            completion: Completion::Merged { .. }
        })
    );
    match how {
        RoundEnd::Merged => merged,
        RoundEnd::Approved => merged || matches!(phase, Some(ItemPhase::Approved { .. })),
        RoundEnd::Abandoned => true,
    }
}

/// Checks the rounds of `team` against its items and its runs.
pub(super) fn check_rounds(team: &Team) -> Result<(), TeamError> {
    check_round_list(team).map_err(|source| TeamError::Rounds {
        team: team.id.clone(),
        source,
    })
}

/// Checks the rounds of `team`, then its items for rounds, then its runs.
fn check_round_list(team: &Team) -> Result<(), RoundsError> {
    let mut seen: HashSet<&RoundId> = HashSet::new();
    for round in &team.rounds {
        if !seen.insert(round.id()) {
            return Err(RoundsError::DuplicateRound {
                round: round.id().clone(),
            });
        }
        check_round(team, round)?;
    }
    for progress in &team.items {
        let item = progress.id();
        let mut rounds = team.rounds.iter().filter(|round| round.item() == Some(item));
        let first = rounds.next();
        if let (Some(first), Some(second)) = (first, rounds.next()) {
            return Err(RoundsError::ItemWithTwoRounds {
                item,
                first: first.id().clone(),
                second: second.id().clone(),
            });
        }
        let listed = matches!(
            progress.phase(),
            ItemPhase::Done {
                completion: Completion::Listed
            }
        );
        if first.is_none() && !listed && !matches!(progress.phase(), ItemPhase::Pending) {
            return Err(RoundsError::ItemWithoutRound {
                item,
                phase: progress.phase().kind(),
            });
        }
    }
    check_runs(team)
}

/// Checks one round of `team` on its own: its item, and how it ended.
fn check_round(team: &Team, round: &Round) -> Result<(), RoundsError> {
    let Some(item) = round.item() else {
        if !team.items.is_empty() {
            return Err(RoundsError::NoItemInTeamWithItems {
                round: round.id().clone(),
            });
        }
        if let Some(first) = team.rounds.iter().find(|other| other.id() != round.id()) {
            return Err(RoundsError::WholeWorkTwice {
                first: first.id().clone(),
                second: round.id().clone(),
            });
        }
        return check_end(round, None);
    };
    let Some(progress) = team.item(item) else {
        return Err(RoundsError::UnknownItem {
            round: round.id().clone(),
            item,
        });
    };
    if matches!(
        progress.phase(),
        ItemPhase::Done {
            completion: Completion::Listed
        }
    ) {
        return Err(RoundsError::RoundOfListedItem {
            round: round.id().clone(),
            item,
        });
    }
    check_end(round, Some(progress.phase()))
}

/// Checks that the way `round` ended fits the phase of its item.
fn check_end(round: &Round, phase: Option<&ItemPhase>) -> Result<(), RoundsError> {
    match round.ended() {
        Some(how) if !end_fits(how, phase) => Err(RoundsError::EndDoesNotFit {
            round: round.id().clone(),
            how,
            item: phase.map(ItemPhase::kind),
        }),
        Some(_) | None => Ok(()),
    }
}

/// Checks that every run of every task of `team` names only rounds the team has.
fn check_runs(team: &Team) -> Result<(), RoundsError> {
    let members = team
        .crew
        .roster()
        .into_iter()
        .map(|(_, member)| member)
        .chain(team.reviewers.iter());
    for member in members {
        check_member_runs(team, member)?;
    }
    Ok(())
}

/// Checks that every run of `member` names only rounds `team` has.
fn check_member_runs(team: &Team, member: &Member) -> Result<(), RoundsError> {
    for run in member.runs() {
        if let Some(unknown) = run.rounds().iter().find(|named| team.round(named).is_none()) {
            return Err(RoundsError::RunNamesUnknownRound {
                task: member.task().clone(),
                run: run.id().clone(),
                round: unknown.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
