//! The pull request of the item a task works for and its review, as the owner reads them from the state for the briefing
//! of a new session: the item's phase, its fix passes and its findings ledger.

use std::num::NonZeroU32;

use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::state::{ItemPhase, ItemProgress, PullRequest, RecordedFinding, Round};
use agent_runtime::run::compose::{BriefingFindings, BriefingPullRequest, BriefingReview, ReviewStanding};

use super::BriefingProblem;
use crate::daemon::owner::Owner;
use crate::daemon::team::Why;

/// What the owner knows of the review besides the item: the rules and the clock of the installation.
#[derive(Debug, Clone, Copy)]
pub(super) struct ReviewSetting<'a> {
    /// The configured number of fix passes, `[review] fix_passes`.
    pub(super) limit: u32,
    /// The zone the briefing shows times in.
    pub(super) zone: &'a Zone,
    /// Whether the lead was sent back as often as it is and waits for the human.
    pub(super) waits_for_human: bool,
}

/// The pull request pointed at by [`ReviewFacts`], with its time already written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRequestFacts {
    /// Its number in the repository.
    number: NonZeroU32,
    /// The branch it merges.
    branch: String,
    /// When it was opened, as the zone of the host writes it.
    opened: String,
}

/// What the reviews of the pull request recorded, as the state holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FindingsFacts {
    /// The numbered findings, in the order of their numbers.
    Ledger(Vec<RecordedFinding>),
    /// A review of before the findings were numbered, of this pull request.
    Legacy(NonZeroU32),
}

/// The pull request of the task's item and its review, owned, so that the briefing can borrow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::owner) struct ReviewFacts {
    /// The pull request, when the item has one.
    pull_request: Option<PullRequestFacts>,
    /// Where its review stands.
    standing: ReviewStanding,
    /// What its reviews recorded.
    findings: FindingsFacts,
}

impl Default for ReviewFacts {
    /// An item with no pull request.
    fn default() -> Self {
        Self {
            pull_request: None,
            standing: ReviewStanding::NotInReview,
            findings: FindingsFacts::Ledger(Vec::new()),
        }
    }
}

/// A pass number counted from 1; a count that does not fit is the largest number, a zero count the first pass.
fn pass_of(count: usize) -> NonZeroU32 {
    NonZeroU32::new(u32::try_from(count).unwrap_or(u32::MAX)).unwrap_or(NonZeroU32::MIN)
}

impl ReviewFacts {
    /// The review of `item`, the item of the task's open round; an item with no pull request, or no item, has none.
    pub(super) fn of(item: Option<&ItemProgress>, setting: ReviewSetting<'_>) -> Self {
        let Some(item) = item else {
            return Self::default();
        };
        let (pr, standing) = match item.phase() {
            ItemPhase::InReview { pr, .. } => (
                pr,
                ReviewStanding::InReview {
                    pass: pass_of(item.completed_review_passes().saturating_add(1)),
                },
            ),
            ItemPhase::Fixing { pr, .. } => {
                let pass = pass_of(item.fix_passes().len());
                let limit = setting.limit;
                let standing = if setting.waits_for_human {
                    ReviewStanding::WaitingForHuman { pass, limit }
                } else {
                    ReviewStanding::Fixing { pass, limit }
                };
                (pr, standing)
            }
            ItemPhase::Approved { pr, .. } => (pr, ReviewStanding::Approved),
            ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => return Self::default(),
        };
        let findings = match item.findings() {
            Some(ledger) => FindingsFacts::Ledger(ledger.entries().to_vec()),
            None => FindingsFacts::Legacy(pr.number.get()),
        };
        Self {
            pull_request: Some(pull_request_of(pr, setting.zone)),
            standing,
            findings,
        }
    }

    /// The review as the briefing takes it.
    pub(super) fn as_review(&self) -> BriefingReview<'_> {
        BriefingReview {
            pull_request: self.pull_request.as_ref().map(|pr| BriefingPullRequest {
                number: pr.number,
                branch: &pr.branch,
                opened: &pr.opened,
            }),
            standing: self.standing,
            findings: match &self.findings {
                FindingsFacts::Ledger(ledger) => BriefingFindings::Ledger(ledger),
                FindingsFacts::Legacy(pull_request) => BriefingFindings::Legacy {
                    pull_request: *pull_request,
                },
            },
        }
    }
}

/// The pull request `pr` with its time written in `zone`.
fn pull_request_of(pr: &PullRequest, zone: &Zone) -> PullRequestFacts {
    PullRequestFacts {
        number: pr.number.get(),
        branch: pr.branch.to_string(),
        opened: zone.stamp_minute(pr.opened_at),
    }
}

/// Whether the review says the lead waits for the human: its last correction ended and the human was told
/// (`held_for_the_human`), and the run that starts is not a correction itself. The counter of send-backs reaches its limit
/// when the last correction is released, before it ran, so only the lead the human was told about waits.
pub(super) fn waits_for_human(held_for_the_human: bool, why: Option<&Why>) -> bool {
    held_for_the_human && !matches!(why, Some(Why::Gate(_)))
}

impl Owner {
    /// The review a run is briefed with: that of [`Owner::briefing_review`] when the run starts a new session (`briefed`),
    /// and none for a run that keeps its session.
    ///
    /// # Errors
    /// [`BriefingProblem`] when the task or its rounds cannot be told.
    pub(in crate::daemon::owner) fn briefed_review(
        &self,
        task: &TaskId,
        why: Option<&Why>,
        briefed: bool,
    ) -> Result<ReviewFacts, BriefingProblem> {
        if briefed {
            self.briefing_review(task, why)
        } else {
            Ok(ReviewFacts::default())
        }
    }

    /// The review of the item of the open round `task` works for, for the run that starts for `why`; none when it works for
    /// no round, or the round's item has no pull request yet.
    ///
    /// # Errors
    /// [`BriefingProblem`] when the task or its rounds cannot be told.
    pub(in crate::daemon::owner) fn briefing_review(
        &self,
        task: &TaskId,
        why: Option<&Why>,
    ) -> Result<ReviewFacts, BriefingProblem> {
        let Some(round) = self.open_round_of(task)? else {
            return Ok(ReviewFacts::default());
        };
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| BriefingProblem::NoTask { task: task.clone() })?;
        let item = found
            .team
            .round(&round)
            .and_then(Round::item)
            .and_then(|id| found.team.item(id));
        let setting = ReviewSetting {
            limit: self.parts.settings.config.review().fix_passes,
            zone: &self.parts.zone,
            waits_for_human: waits_for_human(self.held_for_the_human(task), why),
        };
        Ok(ReviewFacts::of(item, setting))
    }
}

#[cfg(test)]
mod tests;
