//! Durable investigator assignments, separate from the mutable progress of an item.

use super::Team;
use crate::ids::{InvestigatorName, RoundId, TaskId};
use crate::kinds::ActivityKind;
use crate::state::{
    CheckedMut, ItemPhase, Member, OpinionFailure, Participant, ReviewOutcome, RunEnd, RunState, Timestamp,
};
use crate::status::TaskStatus;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

mod opinions;

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
        match self {
            Self::TeamMember => ActivityKind::TeamMember,
            Self::FirstReview | Self::Verification { .. } | Self::SecondOpinion { .. } => ActivityKind::Reviewer,
            Self::Investigation => ActivityKind::Investigator,
            Self::TaskAllocation => ActivityKind::TaskAllocator,
        }
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
        &self.activities
    }

    /// Retains assignments and outcomes currently known by the team, without changing reviewer selection.
    pub fn record_activities(&mut self, at: Timestamp) {
        let mut found = Vec::new();
        for (_, member) in self.crew.roster() {
            if let Participant::Investigator(investigator) = member.agent() {
                found.push(Activity {
                    investigator: investigator.clone(),
                    task: member.task().clone(),
                    rounds: self.rounds.iter().map(|round| round.id().clone()).collect(),
                    request: ActivityRequest::TeamMember,
                    asked_at: self.created_at,
                    outcome: ActivityOutcome::Working,
                });
            }
        }
        for item in &self.items {
            let rounds: Vec<_> = self
                .rounds
                .iter()
                .filter(|round| round.item() == Some(item.id()))
                .map(|round| round.id().clone())
                .collect();
            found.extend(self.opinion_activities(item, &rounds, at));
            let mut pass: Option<NonZeroU32> = None;
            let mut reviews = |reviews: &[crate::state::Review]| {
                let request = pass.map_or(ActivityRequest::FirstReview, |pass| ActivityRequest::Verification {
                    pass,
                });
                for review in reviews {
                    found.push(Activity {
                        investigator: review.reviewer.clone(),
                        task: review.task.clone(),
                        rounds: rounds.clone(),
                        request: request.clone(),
                        asked_at: review.requested_at,
                        outcome: match review.outcome {
                            ReviewOutcome::Pending => ActivityOutcome::Working,
                            ReviewOutcome::Submitted { at, .. } => ActivityOutcome::Finished {
                                at,
                                end: ActivityEnd::Reported,
                            },
                        },
                    });
                }
                pass = Some(pass.map_or(NonZeroU32::MIN, |n| n.saturating_add(1)));
            };
            for fixed in item.fix_passes() {
                reviews(&fixed.requested_by);
            }
            match item.phase() {
                ItemPhase::InReview { reviews: current, .. } | ItemPhase::Approved { reviews: current, .. } => {
                    reviews(current)
                }
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Done { .. } => {}
            }
        }
        for candidate in found {
            self.retain_activity(candidate);
        }
        let endings: Vec<_> = self
            .activities
            .iter()
            .map(|entry| {
                self.find(&entry.task)
                    .and_then(|(_, member)| terminal(member, entry, at))
            })
            .collect();
        for (entry, ending) in self.activities.iter_mut().zip(endings) {
            if let Some(ending) = ending {
                entry.outcome = ending;
            }
        }
    }

    /// Combines copies of an assignment shared by several items without losing a completed report.
    fn retain_activity(&mut self, candidate: Activity) {
        if let Some(known) = self.activities.iter_mut().find(|known| {
            known.investigator == candidate.investigator
                && known.task == candidate.task
                && known.request == candidate.request
                && known.asked_at == candidate.asked_at
        }) {
            for round in candidate.rounds {
                if !known.rounds.contains(&round) {
                    known.rounds.push(round);
                }
            }
            let already_reported = matches!(
                known.outcome,
                ActivityOutcome::Finished {
                    end: ActivityEnd::Reported,
                    ..
                }
            );
            if !already_reported
                && (matches!(known.outcome, ActivityOutcome::Working)
                    || matches!(
                        candidate.outcome,
                        ActivityOutcome::Finished {
                            end: ActivityEnd::Reported,
                            ..
                        }
                    ))
            {
                known.outcome = candidate.outcome;
            }
        } else {
            self.activities.push(candidate);
        }
    }
}

/// A terminal task closes only outstanding assignments; a later stop cannot overwrite an accepted report.
fn terminal(member: &Member, entry: &Activity, now: Timestamp) -> Option<ActivityOutcome> {
    if !matches!(entry.outcome, ActivityOutcome::Working) {
        return None;
    }
    let last = member.runs().last().and_then(|run| match run.state() {
        RunState::Ended { at, end } if *at >= entry.asked_at => Some((*at, end.clone())),
        RunState::Ended { .. } | RunState::Running => None,
    });
    let at = last.as_ref().map_or(now.max(entry.asked_at), |(at, _)| *at);
    let end = match member.status() {
        TaskStatus::Stopped => ActivityEnd::Stopped,
        TaskStatus::Failed => ActivityEnd::Failed {
            reason: last.map(|(_, end)| end),
        },
        TaskStatus::Done if entry.request == ActivityRequest::TeamMember => ActivityEnd::Reported,
        TaskStatus::Done
        | TaskStatus::Queued
        | TaskStatus::Running
        | TaskStatus::Waiting
        | TaskStatus::Questions
        | TaskStatus::Paused => return None,
    };
    Some(ActivityOutcome::Finished { at, end })
}

impl CheckedMut<'_, Team> {
    /// See [`Team::record_activities`].
    pub fn record_activities(&mut self, at: Timestamp) {
        self.value.record_activities(at);
    }
}

#[cfg(test)]
mod tests;
