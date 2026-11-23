//! Rebuilds a team's work for a fresh crew while retaining only completed rounds.

use super::{Crew, Team, TeamError};
use crate::ids::TeamId;
use crate::state::Timestamp;

impl Team {
    /// Builds a new team with the same task list and merged progress, with fresh member sessions.
    ///
    /// IDs and the creation time are supplied once by the caller and persisted before external effects.
    /// No runs, reviews, questions or unmerged rounds are resumed. Approved split definitions remain available.
    ///
    /// # Errors
    /// Returns [`TeamError`] if the reconstructed progress does not satisfy the team's invariants.
    pub fn recovery_team(&self, id: TeamId, created_at: Timestamp, crew: Crew) -> Result<Self, TeamError> {
        Ok(Self::new(
            id,
            self.repo.clone(),
            created_at,
            self.merge,
            self.task_list.clone(),
            crew,
        ))
    }
}

#[cfg(test)]
mod tests;
