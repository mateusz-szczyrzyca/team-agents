//! Durable recovery intents and repeatable installation into the state; no external effects.

use super::{Crew, InvalidState, State, Team, TeamError, Timestamp};
use crate::decide::restart::{self, RestartCount};
use crate::ids::TeamId;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Stable identities selected before any worktree or bus operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryDestination {
    /// Fresh replacement team identity.
    pub id: TeamId,
    /// Time chosen once for the replacement.
    pub created_at: Timestamp,
    /// Fresh task identities and selected participants.
    pub crew: Crew,
}

/// Whether the replacement has been inserted into the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPhase {
    /// Intent saved before removal; external cleanup and creation remain the caller's responsibility.
    Pending,
    /// Installation completed; replay must never recreate a subsequently removed team.
    Installed,
}

mod wire;
pub(super) use wire::read_ledger;

/// A saved source snapshot and its immutable replacement plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recovery {
    /// Original work and context, retained after removal.
    source: Team,
    /// Exact team to install, including stable team and task identities.
    replacement: Team,
    /// Restart allowance consumed by this reservation in its lineage.
    ordinal: NonZeroU32,
    /// State-side installation progress.
    phase: RecoveryPhase,
}

impl Recovery {
    /// Original team snapshot for reconciliation and a recovery briefing.
    #[must_use]
    pub const fn source(&self) -> &Team {
        &self.source
    }
    /// Immutable replacement plan for repeatable external setup.
    #[must_use]
    pub const fn replacement(&self) -> &Team {
        &self.replacement
    }
    /// Restart allowance already reserved, including this replacement.
    #[must_use]
    pub const fn ordinal(&self) -> NonZeroU32 {
        self.ordinal
    }
    /// Whether state-side installation has completed.
    #[must_use]
    pub const fn phase(&self) -> RecoveryPhase {
        self.phase
    }
}

/// Result of planning a replacement without performing external effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryPreparation {
    /// A stable destination was reserved, or the earlier reservation was returned.
    Reserved(TeamId),
    /// The configured allowance is exhausted; no reservation was written.
    Exhausted,
}

/// Result of applying a saved replacement plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryInstallation {
    /// Replacement inserted and intent marked installed in the same state transition.
    Installed,
    /// Earlier installation is retained without changing any current team or progress.
    AlreadyInstalled,
}

/// Why a recovery intent cannot be prepared or installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryError {
    /// No source team or saved intent is available.
    #[error("recovery source {team} is unknown")]
    Unknown {
        /// Requested source identity.
        team: TeamId,
    },
    /// The source must be removed before state installation.
    #[error("recovery source {team} is still present")]
    SourcePresent {
        /// Source still in the active state.
        team: TeamId,
    },
    /// A reused identity does not continue an installed replacement's task list.
    #[error("recovery source {team} does not continue its installed predecessor")]
    InvalidPredecessor {
        /// Source whose prior installation or task list is inconsistent.
        team: TeamId,
    },
    /// Reconstruction failed the team's existing invariant checks.
    #[error("cannot reconstruct recovery team")]
    Team {
        /// Underlying invariant failure.
        source: TeamError,
    },
    /// Replacement identities conflict with the active state.
    #[error("cannot insert recovery team")]
    Insert {
        /// Underlying identity conflict.
        source: InvalidState,
    },
}

impl State {
    /// Saved intent for a source team, including after its removal.
    #[must_use]
    pub fn recovery(&self, source: &TeamId) -> Option<&Recovery> {
        self.recoveries.iter().find(|record| record.source.id() == source)
    }

    /// Reserves stable replacement identities and a restart ordinal without external effects.
    ///
    /// The caller must persist this state before removing the source or creating resources.
    /// Repeating a reservation returns its original destination, irrespective of a new proposal.
    /// # Errors
    /// Returns [`RecoveryError`] when the source is unknown or reconstruction is invalid.
    pub fn prepare_recovery(
        &mut self,
        source: &TeamId,
        destination: RecoveryDestination,
        limit: u32,
    ) -> Result<RecoveryPreparation, RecoveryError> {
        if let Some(record) = self.recovery(source) {
            return Ok(RecoveryPreparation::Reserved(record.replacement.id().clone()));
        }
        let source = self
            .team(source)
            .ok_or_else(|| RecoveryError::Unknown { team: source.clone() })?;
        let used = self
            .recoveries
            .iter()
            .find(|record| record.replacement.id() == source.id())
            .map_or(RestartCount(0), |record| RestartCount(record.ordinal.get()));
        let restart::Decision::Restart(ordinal) = restart::decide(used, limit) else {
            return Ok(RecoveryPreparation::Exhausted);
        };
        let replacement = source
            .recovery_team(destination.id, destination.created_at, destination.crew)
            .map_err(|source| RecoveryError::Team { source })?;
        check_destination(
            &replacement,
            self.teams.iter().chain(
                self.recoveries
                    .iter()
                    .flat_map(|record| [&record.source, &record.replacement]),
            ),
        )
        .map_err(|source| RecoveryError::Insert { source })?;
        let id = replacement.id().clone();
        self.recoveries.push(Recovery {
            source: source.clone(),
            replacement,
            ordinal,
            phase: RecoveryPhase::Pending,
        });
        Ok(RecoveryPreparation::Reserved(id))
    }

    /// Installs a saved replacement after the caller has reconciled external cleanup and setup.
    ///
    /// This operation only changes state. The caller must persist it before starting runs.
    /// # Errors
    /// Returns [`RecoveryError`] for an unknown intent, a still-present source or an identity conflict.
    pub fn install_recovery(&mut self, source: &TeamId) -> Result<RecoveryInstallation, RecoveryError> {
        let record = self
            .recovery(source)
            .ok_or_else(|| RecoveryError::Unknown { team: source.clone() })?;
        if record.phase == RecoveryPhase::Installed {
            return Ok(RecoveryInstallation::AlreadyInstalled);
        }
        if self.team(source).is_some() {
            return Err(RecoveryError::SourcePresent { team: source.clone() });
        }
        let replacement = record.replacement.clone();
        self.insert_team(replacement)
            .map_err(|source| RecoveryError::Insert { source })?;
        for record in &mut self.recoveries {
            if record.source.id() == source {
                record.phase = RecoveryPhase::Installed;
            }
        }
        Ok(RecoveryInstallation::Installed)
    }
}

/// Checks fresh team and task identities against active and historically reserved teams.
fn check_destination<'a>(replacement: &Team, occupied: impl Iterator<Item = &'a Team>) -> Result<(), InvalidState> {
    let tasks: std::collections::HashSet<_> = super::team_members(replacement).map(super::Member::task).collect();
    for other in occupied {
        if other.id() == replacement.id() {
            return Err(InvalidState::DuplicateTeam {
                team: replacement.id().clone(),
            });
        }
        for member in super::team_members(other) {
            if tasks.contains(member.task()) {
                return Err(InvalidState::DuplicateTask {
                    task: member.task().clone(),
                    first: other.id().clone(),
                    second: replacement.id().clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
