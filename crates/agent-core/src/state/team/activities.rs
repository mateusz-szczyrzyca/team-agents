//! Durable investigator assignments, separate from the mutable progress of an item.

use super::Team;
use crate::ids::{InvestigatorName, RoundId, TaskId};
use crate::kinds::ActivityKind;
use crate::state::{
    CheckedMut, Completion, ItemPhase, ItemProgress, Member, OpinionFailure, Participant, PrNumber, ReviewOutcome,
    RunEnd, RunState, Timestamp,
};
use crate::status::{RunOutcome, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

    /// What distinguishes assignments of one task asked at one time.
    const fn subject(&self) -> ActivitySubject {
        match self {
            Self::TeamMember => ActivitySubject::TeamMember,
            Self::FirstReview => ActivitySubject::Review { pass: 0 },
            Self::Verification { pass } => ActivitySubject::Review { pass: pass.get() },
            Self::SecondOpinion { pass, finding } => ActivitySubject::SecondOpinion {
                pass: *pass,
                finding: *finding,
            },
            Self::Investigation => ActivitySubject::Investigation,
            Self::TaskAllocation => ActivitySubject::TaskAllocation,
        }
    }
}

/// What an assignment is about, for telling the copies of one assignment from different ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ActivitySubject {
    /// See [`ActivityRequest::TeamMember`].
    TeamMember,
    /// A first review (pass 0) or the verification after the given fix pass of the pull request.
    Review {
        /// The fix passes of the pull request before this review.
        pass: u32,
    },
    /// See [`ActivityRequest::SecondOpinion`].
    SecondOpinion {
        /// The fix pass whose answer requested the opinion.
        pass: NonZeroU32,
        /// The finding number.
        finding: NonZeroU32,
    },
    /// See [`ActivityRequest::Investigation`].
    Investigation,
    /// See [`ActivityRequest::TaskAllocation`].
    TaskAllocation,
}

/// The identity of an assignment: who, in which task, asked when and about what.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActivityKey {
    /// The investigator asked.
    investigator: InvestigatorName,
    /// Its task.
    task: TaskId,
    /// When it was asked.
    asked_at: Timestamp,
    /// What it was asked about.
    subject: ActivitySubject,
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
#[serde(try_from = "ActivityWire")]
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

/// Untrusted activity fields, checked before they become a retained assignment.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityWire {
    /// The investigator named by the record.
    investigator: InvestigatorName,
    /// The task assigned to it.
    task: TaskId,
    /// Rounds served, without duplicates.
    rounds: Vec<RoundId>,
    /// The assignment's classification and subject.
    request: ActivityRequest,
    /// Earliest permitted ending time.
    asked_at: Timestamp,
    /// The recorded progress or terminal outcome.
    outcome: ActivityOutcome,
}

/// Inconsistent timestamps or round references in a persisted activity.
#[derive(Debug, thiserror::Error)]
enum ActivityInputError {
    /// Ending before assignment would make chronological status misleading.
    #[error("activity for task {task} ends before it was assigned")]
    EndsBeforeAssignment {
        /// The inconsistent task.
        task: TaskId,
    },
    /// One assignment serves any given round at most once.
    #[error("activity for task {task} repeats round {round}")]
    RepeatedRound {
        /// The inconsistent task.
        task: TaskId,
        /// The duplicate reference.
        round: RoundId,
    },
}

impl TryFrom<ActivityWire> for Activity {
    type Error = ActivityInputError;

    fn try_from(wire: ActivityWire) -> Result<Self, Self::Error> {
        if matches!(wire.outcome, ActivityOutcome::Finished { at, .. } if at < wire.asked_at) {
            return Err(ActivityInputError::EndsBeforeAssignment { task: wire.task });
        }
        let mut seen = std::collections::HashSet::new();
        for round in &wire.rounds {
            if !seen.insert(round) {
                return Err(ActivityInputError::RepeatedRound {
                    task: wire.task,
                    round: round.clone(),
                });
            }
        }
        Ok(Self {
            investigator: wire.investigator,
            task: wire.task,
            rounds: wire.rounds,
            request: wire.request,
            asked_at: wire.asked_at,
            outcome: wire.outcome,
        })
    }
}

impl Activity {
    /// The identity under which copies of one assignment are combined.
    fn key(&self) -> ActivityKey {
        ActivityKey {
            investigator: self.investigator.clone(),
            task: self.task.clone(),
            asked_at: self.asked_at,
            subject: self.request.subject(),
        }
    }

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

    /// Ends the outstanding assignments of `task` as failed at `at`, with the reason of a run that ended without a status:
    /// the daemon could not start the run the task was due.
    pub fn fail_activities_before_a_run(&mut self, task: &TaskId, at: Timestamp) {
        for entry in &mut self.activities {
            if &entry.task == task && matches!(entry.outcome, ActivityOutcome::Working) {
                entry.outcome = ActivityOutcome::Finished {
                    at: at.max(entry.asked_at),
                    end: ActivityEnd::Failed {
                        reason: Some(RunEnd::NoStatus { exit_code: None }),
                    },
                };
            }
        }
    }

    /// The fix passes of the pull request of `item` that came before the item joined it: the most passes any item on the
    /// pull request has had, less the item's own. Zero for an item that has no pull request.
    fn passes_before_joining(&self, item: &ItemProgress) -> u32 {
        let Some(pull) = pull_of(item) else {
            return 0;
        };
        let most = self
            .items
            .iter()
            .filter(|other| pull_of(other) == Some(pull))
            .map(|other| other.fix_passes().len())
            .max()
            .unwrap_or(0);
        u32::try_from(most.saturating_sub(item.fix_passes().len())).unwrap_or(u32::MAX)
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
            let mut item_found = Vec::new();
            // An item that joined a pull request after some fix passes counts only the passes since: the pull
            // request's pass of its first review is the difference to the most any item on it has had.
            let mut pass = NonZeroU32::new(self.passes_before_joining(item));
            let mut reviews = |reviews: &[crate::state::Review]| {
                let request = pass.map_or(ActivityRequest::FirstReview, |pass| ActivityRequest::Verification {
                    pass,
                });
                for review in reviews {
                    item_found.push(Activity {
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
                    reviews(current);
                }
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Done { .. } => {}
            }
            item_found.extend(self.opinion_activities(item, &rounds, at));
            found.extend(item_found);
        }
        let mut index: HashMap<ActivityKey, usize> = self
            .activities
            .iter()
            .enumerate()
            .map(|(position, known)| (known.key(), position))
            .collect();
        for candidate in found {
            self.retain_activity(&mut index, candidate);
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

    /// Combines copies of an assignment shared by several items without losing a completed report; `index` finds the
    /// retained assignments by identity and gains the new one.
    fn retain_activity(&mut self, index: &mut HashMap<ActivityKey, usize>, candidate: Activity) {
        let Some(known) = index.get(&candidate.key()).and_then(|at| self.activities.get_mut(*at)) else {
            index.insert(candidate.key(), self.activities.len());
            self.activities.push(candidate);
            return;
        };
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
    }
}

/// The pull request an item is reviewed, fixed or approved on, or was merged with.
fn pull_of(item: &ItemProgress) -> Option<PrNumber> {
    match item.phase() {
        ItemPhase::InReview { pr, .. }
        | ItemPhase::Fixing { pr, .. }
        | ItemPhase::Approved { pr, .. }
        | ItemPhase::Done {
            completion: Completion::Merged { pr, .. },
        } => Some(pr.number),
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::Done {
            completion: Completion::Listed,
        } => None,
    }
}

/// Whether a run that ended like this can have failed its task by itself. The other ends leave the task alive (it slept,
/// finished, waits for the quota or was stopped), so a task that is failed after one of them failed to start.
const fn can_fail_task(end: &RunEnd) -> bool {
    match end {
        RunEnd::Reported { outcome } => match outcome {
            RunOutcome::Failed => true,
            RunOutcome::Done | RunOutcome::Questions | RunOutcome::Sleeping | RunOutcome::Excluded => false,
        },
        RunEnd::NoStatus { .. } | RunEnd::Hung | RunEnd::TurnLimit => true,
        RunEnd::QuotaExhausted { .. } | RunEnd::RateLimited | RunEnd::Stopped => false,
    }
}

/// A terminal task closes only outstanding assignments; a later stop cannot overwrite an accepted report.
///
/// The end of the last run since the assignment is the time of the ending only when that run caused the task's status:
/// a failure or a completion is a run's end, a stop is one only when the run was stopped, a failure only when the run
/// ended in a way that can fail a task ([`can_fail_task`]). A task stopped while it waited, or failed without a run
/// that could have failed it (it never started, or it slept and could not be resumed), ends at `now`, when the daemon saw
/// it; a start failure has the reason the daemon gives it, a run that ended without a status.
fn terminal(member: &Member, entry: &Activity, now: Timestamp) -> Option<ActivityOutcome> {
    if !matches!(entry.outcome, ActivityOutcome::Working) {
        return None;
    }
    let last = member.runs().last().and_then(|run| match run.state() {
        RunState::Ended { at, end } if *at >= entry.asked_at => Some((*at, end)),
        RunState::Ended { .. } | RunState::Running => None,
    });
    let seen = now.max(entry.asked_at);
    let (at, end) = match (member.status(), last) {
        (TaskStatus::Stopped, Some((at, RunEnd::Stopped))) => (at, ActivityEnd::Stopped),
        (TaskStatus::Stopped, _) => (seen, ActivityEnd::Stopped),
        (TaskStatus::Failed, Some((at, end))) if can_fail_task(end) => (
            at,
            ActivityEnd::Failed {
                reason: Some(end.clone()),
            },
        ),
        (TaskStatus::Failed, _) => (
            seen,
            ActivityEnd::Failed {
                reason: Some(RunEnd::NoStatus { exit_code: None }),
            },
        ),
        (TaskStatus::Done, last) if entry.request == ActivityRequest::TeamMember => {
            (last.map_or(seen, |(at, _)| at), ActivityEnd::Reported)
        }
        (
            TaskStatus::Done
            | TaskStatus::Queued
            | TaskStatus::Running
            | TaskStatus::Waiting
            | TaskStatus::Questions
            | TaskStatus::Paused,
            _,
        ) => return None,
    };
    Some(ActivityOutcome::Finished { at, end })
}

impl CheckedMut<'_, Team> {
    /// See [`Team::fail_activities_before_a_run`].
    pub fn fail_activities_before_a_run(&mut self, task: &TaskId, at: Timestamp) {
        self.value.fail_activities_before_a_run(task, at);
    }

    /// See [`Team::record_activities`].
    pub fn record_activities(&mut self, at: Timestamp) {
        self.value.record_activities(at);
    }
}

#[cfg(test)]
mod tests;
