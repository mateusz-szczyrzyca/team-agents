//! Durable retry history and the start of the current investigation attempt.

use std::num::NonZeroU32;

use super::Team;
use crate::ids::{InvestigatorName, TaskId};
use crate::state::{CheckedMut, Timestamp};

/// One request's current assignment and the investigators that failed it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationRecord {
    /// Stable bus request identifier.
    pub id: NonZeroU32,
    /// Task created for the current attempt.
    pub task: TaskId,
    /// Current task creation time; queue time never counts against its deadline.
    pub started: Timestamp,
    /// Investigators excluded from subsequent attempts of this request.
    pub failed_with: Vec<InvestigatorName>,
}

impl Team {
    /// Requests assigned to this team, including their retry exclusions.
    #[must_use]
    pub fn investigations(&self) -> &[InvestigationRecord] {
        &self.investigations
    }
}

impl CheckedMut<'_, Team> {
    /// Records the current assignment, replacing the previous attempt of that request.
    pub fn record_investigation(&mut self, record: InvestigationRecord) {
        if let Some(previous) = self
            .value
            .investigations
            .iter_mut()
            .find(|previous| previous.id == record.id)
        {
            *previous = record;
        } else {
            self.value.investigations.push(record);
        }
    }
}
