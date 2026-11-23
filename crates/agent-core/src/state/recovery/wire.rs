//! Validates saved recovery plans and their ordered restart lineage at the serialization boundary.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer};

use super::{Recovery, RecoveryPhase, Team, check_destination};
use crate::ids::TeamId;
use crate::state::{InvalidState, TeamError, strict::checked, team_members};

/// Private mirror; no unchecked public deserializer can bypass plan validation.
#[derive(Deserialize)]
#[serde(remote = "Recovery", deny_unknown_fields)]
struct RecoveryWire {
    /// Original team snapshot.
    source: Team,
    /// Immutable replacement projection.
    replacement: Team,
    /// Consumed restart allowance.
    ordinal: std::num::NonZeroU32,
    /// State-side installation progress.
    phase: RecoveryPhase,
}

checked!(Recovery, RecoveryWire, check_plan);

/// Invalid recovery data is refused before it enters the domain state.
#[derive(Debug, thiserror::Error)]
enum ReadRecoveryError {
    /// Source progress cannot be projected into a fresh team.
    #[error("cannot reconstruct saved recovery plan")]
    Projection { source: TeamError },
    /// Saved plan differs from the projection of the source snapshot.
    #[error("recovery plan for {team} does not preserve its source")]
    Changed { team: TeamId },
    /// A destination reuses an identity reserved by an earlier team.
    #[error("recovery identities conflict")]
    Identity { source: InvalidState },
    /// Restart number, predecessor state or ordering contradicts the lineage.
    #[error("recovery lineage for {team} is inconsistent")]
    Lineage { team: TeamId },
}

/// Ensures a standalone record cannot alter the source's work or reuse its identities.
fn check_plan(record: &Recovery) -> Result<(), ReadRecoveryError> {
    let target = &record.replacement;
    check_destination(target, std::iter::once(&record.source))
        .map_err(|source| ReadRecoveryError::Identity { source })?;
    let expected = record
        .source
        .recovery_team(target.id().clone(), target.created_at(), target.crew().clone())
        .map_err(|source| ReadRecoveryError::Projection { source })?;
    if expected != *target {
        return Err(ReadRecoveryError::Changed {
            team: record.source.id().clone(),
        });
    }
    Ok(())
}

/// Reads records in reservation order and rejects duplicate sources, cycles and reset counts.
pub(in crate::state) fn read_ledger<'de, D: Deserializer<'de>>(reader: D) -> Result<Vec<Recovery>, D::Error> {
    let records = Vec::<Recovery>::deserialize(reader)?;
    check_lineage(&records).map_err(serde::de::Error::custom)?;
    Ok(records)
}

/// Checks lineage in linear time, also reserving historical team and task identities.
fn check_lineage(records: &[Recovery]) -> Result<(), ReadRecoveryError> {
    let mut sources = HashSet::new();
    let mut identities = HashSet::new();
    let mut targets: HashMap<&TeamId, &Recovery> = HashMap::new();
    let mut tasks = HashMap::new();
    for record in records {
        let source = record.source.id();
        let predecessor = targets.get(source);
        let expected = predecessor.map_or(Some(1), |previous| previous.ordinal.get().checked_add(1));
        if !sources.insert(source)
            || expected != Some(record.ordinal.get())
            || predecessor.is_some_and(|previous| {
                previous.phase != RecoveryPhase::Installed
                    || previous.replacement.repo() != record.source.repo()
                    || previous.replacement.task_list() != record.source.task_list()
            })
        {
            return Err(ReadRecoveryError::Lineage { team: source.clone() });
        }
        identities.insert(source);
        if !identities.insert(record.replacement.id()) {
            return Err(ReadRecoveryError::Lineage { team: source.clone() });
        }
        for member in team_members(&record.source) {
            if tasks.insert(member.task(), source).is_some_and(|owner| owner != source) {
                return Err(ReadRecoveryError::Lineage { team: source.clone() });
            }
        }
        for member in team_members(&record.replacement) {
            if tasks.insert(member.task(), record.replacement.id()).is_some() {
                return Err(ReadRecoveryError::Lineage { team: source.clone() });
            }
        }
        targets.insert(record.replacement.id(), record);
    }
    Ok(())
}
