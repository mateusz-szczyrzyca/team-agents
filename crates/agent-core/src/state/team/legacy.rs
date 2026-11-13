//! A team as schema version 1 wrote it, and the rounds it is given when it is read as version 2.
//!
//! Version 1 had no rounds. Each item that had started gets one, whose ID is derived from the team's ID and the item's
//! place in the list ([`RoundId::derive`]), so that reading the same file twice gives the same state. Version 1 did not
//! record when an item was given out, so a migrated round is opened when its team was started. An item that is merged
//! gets a round closed as `merged` at the time of the merge; one that is approved in a team that does not merge by
//! itself, a round closed as `approved` at its last review; every other item that has started, an open round. An item
//! that is pending, or that the list marked `[x]`, has none. The runs of version 1 name no round.

use std::collections::HashSet;

use serde::Deserialize;

use super::{Crew, MergeMode, TaskListText, Team, TeamError, check_team};
use crate::config::RepoName;
use crate::ids::{RoundId, TeamId};
use crate::state::item::{Completion, ItemPhase, ItemProgress, ReviewOutcome};
use crate::state::member::Member;
use crate::state::round::{ClosedBeforeOpened, Round, RoundEnd};
use crate::state::values::Timestamp;

/// How many IDs are tried for a round before the state is refused: the first is almost always free.
const ID_ATTEMPTS: usize = 8;

/// A team as version 1 wrote it: a [`Team`] without rounds.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::state) struct TeamV1 {
    /// See [`Team`].
    id: TeamId,
    /// See [`Team`].
    repo: RepoName,
    /// See [`Team`].
    created_at: Timestamp,
    /// See [`Team`].
    merge: MergeMode,
    /// See [`Team`].
    task_list: TaskListText,
    /// See [`Team`].
    items: Vec<ItemProgress>,
    /// See [`Team`].
    crew: Crew,
    /// See [`Team`]; a team written without reviewers has none.
    #[serde(default)]
    reviewers: Vec<Member>,
}

/// Why a team of version 1 could not be read as version 2.
#[derive(Debug, thiserror::Error)]
pub(in crate::state) enum LegacyError {
    /// The team's parts do not fit together.
    #[error(transparent)]
    Team(#[from] TeamError),
    /// A round would be closed before it was opened: a time of version 1 that says so.
    #[error(transparent)]
    Time(#[from] ClosedBeforeOpened),
    /// Every ID tried for a round is used by another round.
    #[error("no free ID for the round of item {item} of team {team}")]
    NoFreeId {
        /// The team.
        team: TeamId,
        /// The item.
        item: crate::ids::ItemId,
    },
}

impl Team {
    /// The team `old` as version 2, with the rounds of its items that had started. `used` holds the IDs of the rounds
    /// made so far in the state, and gets the ones made here.
    ///
    /// # Errors
    /// [`LegacyError`]: the team does not fit together, or no free ID is found for a round.
    pub(in crate::state) fn from_v1(old: TeamV1, used: &mut HashSet<RoundId>) -> Result<Self, LegacyError> {
        let TeamV1 {
            id,
            repo,
            created_at,
            merge,
            task_list,
            items,
            crew,
            reviewers,
        } = old;
        let mut rounds = Vec::new();
        for (position, progress) in items.iter().enumerate() {
            let ending = migrated_ending(progress.phase(), merge, created_at);
            if matches!(ending, Ending::NoRound) {
                continue;
            }
            let round_id = free_id(&id, position, items.len(), used).ok_or_else(|| LegacyError::NoFreeId {
                team: id.clone(),
                item: progress.id(),
            })?;
            used.insert(round_id.clone());
            let mut round = Round::opened(round_id, Some(progress.id()), created_at);
            if let Ending::Closed(at, how) = ending {
                round.close(at, how)?;
            }
            rounds.push(round);
        }
        let team = Self {
            id,
            repo,
            created_at,
            merge,
            task_list,
            items,
            rounds,
            crew,
            reviewers,
        };
        check_team(&team)?;
        Ok(team)
    }
}

/// How the round of an item stands after the migration.
enum Ending {
    /// The item has no round: it is pending, or the task list marked it done.
    NoRound,
    /// The round is open.
    Open,
    /// The round is closed at a time, as `how`.
    Closed(Timestamp, RoundEnd),
}

/// How the round of an item in `phase` stands after the migration. A time is never earlier than `created_at`, when the
/// round is opened.
fn migrated_ending(phase: &ItemPhase, merge: MergeMode, created_at: Timestamp) -> Ending {
    match phase {
        ItemPhase::Pending
        | ItemPhase::Done {
            completion: Completion::Listed,
        } => Ending::NoRound,
        ItemPhase::InProgress { .. } | ItemPhase::InReview { .. } | ItemPhase::Fixing { .. } => Ending::Open,
        ItemPhase::Approved { reviews, .. } => match merge {
            MergeMode::Autopilot => Ending::Open,
            MergeMode::NoMerge => {
                let last = reviews
                    .iter()
                    .filter_map(|review| match review.outcome {
                        ReviewOutcome::Submitted { at, .. } => Some(at),
                        ReviewOutcome::Pending => None,
                    })
                    .max();
                Ending::Closed(last.map_or(created_at, |at| at.max(created_at)), RoundEnd::Approved)
            }
        },
        ItemPhase::Done {
            completion: Completion::Merged { at, .. },
        } => Ending::Closed((*at).max(created_at), RoundEnd::Merged),
    }
}

/// An ID for the round at `position` of `count` items of `team` that is not in `used`: the derived ID, or when another
/// team has it, the one derived for the position a whole list later, and so on.
fn free_id(team: &TeamId, position: usize, count: usize, used: &HashSet<RoundId>) -> Option<RoundId> {
    (0..ID_ATTEMPTS).find_map(|attempt| {
        let shifted = attempt.checked_mul(count)?.checked_add(position)?;
        let id = RoundId::derive(team, u32::try_from(shifted).ok()?);
        (!used.contains(&id)).then_some(id)
    })
}
