//! Durable investigator assignments, separate from the mutable progress of an item.

use super::Team;
use crate::ids::{InvestigatorName, RoundId, TaskId};
use crate::kinds::ActivityKind;
use crate::state::{CheckedMut, OpinionFailure, RunEnd, Timestamp};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// What the team asked an investigator to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityRequest {
    /// Join the team's crew as coordinator or member.
    TeamMember,
    /// Review the initial pull request.
    FirstReview,
    /// Verify the corrections made in a fix pass.
    Verification {
        /// The one-based fix pass being verified.
        pass: NonZeroU32,
    },
    /// Independently assess a disputed finding.
    SecondOpinion {
        /// The fix pass whose answer requested this opinion.
        pass: NonZeroU32,
        /// The permanent finding number.
        finding: NonZeroU32,
    },
    /// Answer a question or explain a problem; no daemon workflow starts this yet.
    Investigation,
    /// Divide work or help integrate branches; no daemon workflow starts this yet.
    TaskAllocation,
}

impl ActivityRequest {
    /// The classification of this request, independent of the participant's task role.
    #[must_use]
    pub const fn kind(&self) -> ActivityKind {
        ActivityKind::TeamMember
    }
}

/// How an assignment ended; failures retain the daemon's structured reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityEnd {
    /// The requested report was accepted, or crew work was completed.
    Reported,
    /// The human stopped the task.
    Stopped,
    /// The task entered the failed status.
    Failed {
        /// The last run's outcome, when the daemon has one.
        reason: Option<RunEnd>,
    },
    /// A second-opinion assignment was abandoned or replaced.
    OpinionFailed {
        /// The reason retained in the findings ledger.
        reason: OpinionFailure,
    },
}

/// Whether an assignment is outstanding or has ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityOutcome {
    /// Queued or running, including pauses while the assignment can still resume.
    Working,
    /// The requested work ended.
    Finished {
        /// When the report was accepted or the task ended.
        at: Timestamp,
        /// How it ended.
        end: ActivityEnd,
    },
}

/// One investigator's assignment for a team, shared across items of the same pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activity {
    /// The configured investigator.
    investigator: InvestigatorName,
    /// Its task in this team.
    task: TaskId,
    /// Rounds served by this assignment; crew work may start before any round exists.
    rounds: Vec<RoundId>,
    /// What was asked; its variant also determines the activity kind.
    request: ActivityRequest,
    /// When the assignment was made.
    asked_at: Timestamp,
    /// The durable outcome of the assignment.
    outcome: ActivityOutcome,
}

impl Activity {
    getter!("The investigator asked.", pub const fn investigator(&self) -> &InvestigatorName = &self.investigator);
    getter!("The task assigned.", pub const fn task(&self) -> &TaskId = &self.task);
    getter!("The rounds served.", pub fn rounds(&self) -> &[RoundId] = &self.rounds);
    getter!("What was asked.", pub const fn request(&self) -> &ActivityRequest = &self.request);
    getter!("When it was asked.", pub const fn asked_at(&self) -> Timestamp = self.asked_at);
    getter!("How the assignment stands.", pub const fn outcome(&self) -> &ActivityOutcome = &self.outcome);
}

impl Team {
    /// All investigator assignments, retained after their item progresses or closes.
    #[must_use]
    pub fn activities(&self) -> &[Activity] {
        &[]
    }

    /// Retains assignments and outcomes currently known by the team, without changing reviewer selection.
    pub fn record_activities(&mut self, _at: Timestamp) {}
}

impl CheckedMut<'_, Team> {
    /// See [`Team::record_activities`].
    pub fn record_activities(&mut self, at: Timestamp) {
        self.value.record_activities(at);
    }
}

#[cfg(test)]
mod tests;
