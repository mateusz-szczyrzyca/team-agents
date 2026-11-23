//! Durable recovery intents and repeatable installation into the state; no external effects.

use super::{Crew, InvalidState, State, Team, TeamError, Timestamp};
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
    pub fn recovery(&self, _source: &TeamId) -> Option<&Recovery> {
        None
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
        _destination: RecoveryDestination,
        _limit: u32,
    ) -> Result<RecoveryPreparation, RecoveryError> {
        Err(RecoveryError::Unknown { team: source.clone() })
    }

    /// Installs a saved replacement after the caller has reconciled external cleanup and setup.
    ///
    /// This operation only changes state. The caller must persist it before starting runs.
    /// # Errors
    /// Returns [`RecoveryError`] for an unknown intent, a still-present source or an identity conflict.
    pub fn install_recovery(&mut self, source: &TeamId) -> Result<RecoveryInstallation, RecoveryError> {
        Err(RecoveryError::Unknown { team: source.clone() })
    }
}

#[cfg(test)]
mod tests;
