//! Rebuilds a team's work for a fresh crew while retaining only completed rounds.

use super::{Crew, ItemPhase, Member, RoundEnd, Shape, Team, TeamError, check_team};
use crate::ids::TeamId;
use crate::state::{Completion, Timestamp};

impl Team {
    /// Builds a new team with the same task list and merged progress, with fresh member sessions.
    ///
    /// IDs and the creation time are supplied once by the caller and persisted before external effects.
    /// No runs, reviews, questions or unmerged rounds are resumed. Approved split definitions remain available.
    ///
    /// # Errors
    /// Returns [`TeamError`] if the reconstructed progress does not satisfy the team's invariants.
    pub fn recovery_team(&self, id: TeamId, created_at: Timestamp, crew: Crew) -> Result<Self, TeamError> {
        let mut fresh = Self::new(
            id,
            self.repo.clone(),
            created_at,
            self.merge,
            self.task_list.clone(),
            fresh_crew(crew),
        );
        for (target, source) in fresh.items.iter_mut().zip(&self.items) {
            if matches!(source.phase(), ItemPhase::Done { .. }) {
                *target = source.clone();
            }
        }
        fresh.splits.clone_from(&self.splits);
        fresh.rounds = self
            .rounds
            .iter()
            .filter_map(|round| {
                if round.part().is_none()
                    && let Some(ItemPhase::Done {
                        completion: Completion::Merged { at, .. },
                    }) = round
                        .item()
                        .and_then(|id| self.item(id))
                        .map(super::super::ItemProgress::phase)
                {
                    return Some(round.recovered_merge(*at));
                }
                // Only completed evidence belongs to the replacement; unfinished work gets new rounds.
                (round.part_pull_request().is_some() || round.ended() == Some(RoundEnd::Merged)).then(|| round.clone())
            })
            .collect();
        check_team(&fresh)?;
        Ok(fresh)
    }
}

/// Retains identities and participants but discards sessions and task-local state.
fn fresh_crew(mut crew: Crew) -> Crew {
    let reset = |member: &mut Member| {
        *member = Member::new(member.task().clone(), member.agent().clone());
    };
    match &mut crew.shape {
        Shape::Solo { member } => reset(member),
        Shape::Team {
            coordinator,
            members,
            workers,
        } => {
            reset(coordinator);
            for member in members.iter_mut().chain(workers) {
                reset(member);
            }
        }
    }
    crew
}

#[cfg(test)]
mod tests;
