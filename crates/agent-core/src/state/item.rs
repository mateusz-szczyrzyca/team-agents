//! The progress of a task-list item: who works on it, its pull request, its reviews and fix passes, and its gate
//! result.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::checked::CheckedMut;
use super::gate::GateResult;
use super::strict::{checked, strict_enum};
use super::values::{PrNumber, Timestamp};
use crate::ids::{AgentName, BranchName, InvestigatorName, ItemId, TaskId};
use crate::kinds::keyword_enum;
use crate::status::{ItemStatus, ReviewVerdict};
use crate::task_list::ItemMark;

keyword_enum! {
    /// The phases of an item, which are its [`ItemStatus`]es with the fix pass told apart from the first work.
    ///
    /// Allowed changes ([`PhaseKind::successors`]); each is also a change its [`ItemStatus`] allows:
    ///
    /// | from | to |
    /// |---|---|
    /// | `pending` | `in_progress` |
    /// | `in_progress` | `pending`, `in_review` |
    /// | `in_review` | `fixing`, `approved` |
    /// | `fixing` | `in_review` |
    /// | `approved` | `done` |
    /// | `done` | — |
    pub enum PhaseKind: "item phase" {
        /// Not started.
        Pending => "pending",
        /// A member works on it, or the coordinator integrates it, until its pull request is open.
        InProgress => "in_progress",
        /// Its pull request is open and waits for its two reviews.
        InReview => "in_review",
        /// The reviewers asked for changes: the team fixes them (status `in_progress`).
        Fixing => "fixing",
        /// Both reviews approve; it waits for its merge.
        Approved => "approved",
        /// Merged, or listed as done.
        Done => "done",
    }
}

impl PhaseKind {
    /// The phases an item in this phase may move to.
    #[must_use]
    pub const fn successors(self) -> &'static [Self] {
        match self {
            Self::Pending => &[Self::InProgress],
            Self::InProgress => &[Self::Pending, Self::InReview],
            Self::InReview => &[Self::Fixing, Self::Approved],
            Self::Fixing => &[Self::InReview],
            Self::Approved => &[Self::Done],
            Self::Done => &[],
        }
    }

    /// The item status this phase shows.
    #[must_use]
    pub const fn status(self) -> ItemStatus {
        match self {
            Self::Pending => ItemStatus::Pending,
            Self::InProgress | Self::Fixing => ItemStatus::InProgress,
            Self::InReview => ItemStatus::InReview,
            Self::Approved => ItemStatus::Approved,
            Self::Done => ItemStatus::Done,
        }
    }
}

/// A pull request of an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequest {
    /// Its number in the repository.
    pub number: PrNumber,
    /// The branch it merges: its head.
    pub branch: BranchName,
    /// When it was opened.
    pub opened_at: Timestamp,
}

/// One reviewer's review of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    /// The investigator who reviews; never a member of the reviewed team.
    pub reviewer: InvestigatorName,
    /// The reviewer's task, one of the team's reviewers.
    pub task: TaskId,
    /// When the review was asked for.
    pub requested_at: Timestamp,
    /// Whether the reviewer has answered.
    pub outcome: ReviewOutcome,
}

/// Whether a reviewer has answered.
///
/// Deserialization refuses fields that the variant does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReviewOutcome {
    /// The review is not in yet.
    Pending,
    /// The reviewer reported a verdict.
    Submitted {
        /// The verdict.
        verdict: ReviewVerdict,
        /// When the report came.
        at: Timestamp,
    },
}

/// A fix pass: the reviews that sent an item back to its team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixPass {
    /// The reviews of the round that asked for changes.
    pub requested_by: Vec<Review>,
}

/// How an item came to be done.
///
/// Deserialization refuses fields that the variant does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "how", rename_all = "snake_case")]
pub enum Completion {
    /// The task list marked it `[x]` when the team started.
    Listed,
    /// Its pull request was merged.
    Merged {
        /// The merged pull request.
        pr: PullRequest,
        /// When it was merged.
        at: Timestamp,
    },
}

/// Where an item is, with exactly the data that phase has.
///
/// Deserialization refuses unknown fields and applies [`ItemPhase::check_reviews`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ItemPhase {
    /// Not started.
    Pending,
    /// A member works on it.
    InProgress {
        /// The member the item was assigned to.
        assignee: AgentName,
        /// The member's branch.
        branch: BranchName,
    },
    /// The pull request is open and waits for its reviews.
    InReview {
        /// The pull request.
        pr: PullRequest,
        /// The reviews of this round, in the order asked.
        reviews: Vec<Review>,
    },
    /// The team fixes what the reviewers asked for.
    Fixing {
        /// The member who fixes.
        assignee: AgentName,
        /// The member's branch.
        branch: BranchName,
        /// The pull request the fixes go into.
        pr: PullRequest,
    },
    /// Both reviews approve; the pull request waits to be merged.
    Approved {
        /// The pull request.
        pr: PullRequest,
        /// The reviews that approved it.
        reviews: Vec<Review>,
    },
    /// Finished.
    Done {
        /// How.
        completion: Completion,
    },
}

/// How a [`ReviewOutcome`] is read; see [`strict_enum!`].
#[derive(Deserialize)]
#[serde(
    remote = "ReviewOutcome",
    tag = "status",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum ReviewOutcomeWire {
    /// See [`ReviewOutcome::Pending`].
    Pending,
    /// See [`ReviewOutcome::Submitted`].
    Submitted {
        /// See [`ReviewOutcome::Submitted`].
        verdict: ReviewVerdict,
        /// See [`ReviewOutcome::Submitted`].
        at: Timestamp,
    },
}

/// How a [`Completion`] is read; see [`strict_enum!`].
#[derive(Deserialize)]
#[serde(remote = "Completion", tag = "how", rename_all = "snake_case", deny_unknown_fields)]
enum CompletionWire {
    /// See [`Completion::Listed`].
    Listed,
    /// See [`Completion::Merged`].
    Merged {
        /// See [`Completion::Merged`].
        pr: PullRequest,
        /// See [`Completion::Merged`].
        at: Timestamp,
    },
}

/// How an [`ItemPhase`] is read; see [`strict_enum!`].
#[derive(Deserialize)]
#[serde(remote = "ItemPhase", tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum ItemPhaseWire {
    /// See [`ItemPhase::Pending`].
    Pending,
    /// See [`ItemPhase::InProgress`].
    InProgress {
        /// See [`ItemPhase::InProgress`].
        assignee: AgentName,
        /// See [`ItemPhase::InProgress`].
        branch: BranchName,
    },
    /// See [`ItemPhase::InReview`].
    InReview {
        /// See [`ItemPhase::InReview`].
        pr: PullRequest,
        /// See [`ItemPhase::InReview`].
        reviews: Vec<Review>,
    },
    /// See [`ItemPhase::Fixing`].
    Fixing {
        /// See [`ItemPhase::Fixing`].
        assignee: AgentName,
        /// See [`ItemPhase::Fixing`].
        branch: BranchName,
        /// See [`ItemPhase::Fixing`].
        pr: PullRequest,
    },
    /// See [`ItemPhase::Approved`].
    Approved {
        /// See [`ItemPhase::Approved`].
        pr: PullRequest,
        /// See [`ItemPhase::Approved`].
        reviews: Vec<Review>,
    },
    /// See [`ItemPhase::Done`].
    Done {
        /// See [`ItemPhase::Done`].
        completion: Completion,
    },
}

strict_enum!(ReviewOutcome, ReviewOutcomeWire);
strict_enum!(Completion, CompletionWire);
strict_enum!(ItemPhase, ItemPhaseWire, |_| &[], ItemPhase::check_reviews);

impl ItemPhase {
    /// Which phase this is.
    #[must_use]
    pub const fn kind(&self) -> PhaseKind {
        match self {
            Self::Pending => PhaseKind::Pending,
            Self::InProgress { .. } => PhaseKind::InProgress,
            Self::InReview { .. } => PhaseKind::InReview,
            Self::Fixing { .. } => PhaseKind::Fixing,
            Self::Approved { .. } => PhaseKind::Approved,
            Self::Done { .. } => PhaseKind::Done,
        }
    }

    /// Checks the reviews this phase carries: a round names no reviewer and no task twice, and an approved item
    /// carries at least one review, each an approval.
    ///
    /// # Errors
    /// [`InvalidReviews`] for the first review that does not fit.
    pub fn check_reviews(&self) -> Result<(), InvalidReviews> {
        match self {
            Self::InReview { reviews, .. } => check_round(reviews),
            Self::Approved { reviews, .. } => {
                check_round(reviews)?;
                check_approving(reviews)
            }
            Self::Pending | Self::InProgress { .. } | Self::Fixing { .. } | Self::Done { .. } => Ok(()),
        }
    }
}

/// Whether `review` is a submitted approval.
const fn approves(review: &Review) -> bool {
    matches!(
        review.outcome,
        ReviewOutcome::Submitted {
            verdict: ReviewVerdict::Approve,
            ..
        }
    )
}

/// Checks that `round` names no reviewer and no task twice.
fn check_round(round: &[Review]) -> Result<(), InvalidReviews> {
    let mut reviewers = HashSet::new();
    let mut tasks = HashSet::new();
    for review in round {
        if !reviewers.insert(&review.reviewer) {
            return Err(InvalidReviews::DuplicateReviewer {
                reviewer: review.reviewer.clone(),
            });
        }
        if !tasks.insert(&review.task) {
            return Err(InvalidReviews::DuplicateTask {
                task: review.task.clone(),
            });
        }
    }
    Ok(())
}

/// Checks that `round` is the reviews of an approved item: some, and every one an approval.
fn check_approving(round: &[Review]) -> Result<(), InvalidReviews> {
    if round.is_empty() {
        return Err(InvalidReviews::ApprovedWithoutReviews);
    }
    match round.iter().find(|review| !approves(review)) {
        Some(review) => Err(InvalidReviews::ApprovedNotByAll {
            reviewer: review.reviewer.clone(),
        }),
        None => Ok(()),
    }
}

/// Checks that `round` can be the reviews of a fix pass: a valid, non-empty round that is not wholly approving and
/// contains a submitted request for changes.
fn check_fix_pass(round: &[Review]) -> Result<(), InvalidReviews> {
    check_round(round)?;
    if round.is_empty() {
        return Err(InvalidReviews::FixPassWithoutReviews);
    }
    if round.iter().all(approves) {
        return Err(InvalidReviews::FixPassApproved);
    }
    if !round.iter().any(|review| {
        matches!(
            review.outcome,
            ReviewOutcome::Submitted {
                verdict: ReviewVerdict::RequestChanges,
                ..
            }
        )
    }) {
        return Err(InvalidReviews::FixPassNotRequested);
    }
    Ok(())
}

/// An item's progress: its phase, the fix passes behind it and its last gate result.
///
/// The phase changes only through [`ItemProgress::advance`], which follows [`PhaseKind::successors`].
///
/// Reading checks what the item's own constructors check: a phase with its data ([`ItemPhase`]), a fixing item with
/// a fix pass, and that every fix pass was asked for by reviews that fit it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemProgress {
    /// The item's number in the task list.
    id: ItemId,
    /// Where the item is.
    phase: ItemPhase,
    /// The fix passes so far, oldest first; their number is what the configured limit is compared with.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fix_passes: Vec<FixPass>,
    /// The last gate result for this item, if the gate has run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<GateResult>,
}

/// How an [`ItemProgress`] is read; see [`checked!`].
#[derive(Deserialize)]
#[serde(remote = "ItemProgress", deny_unknown_fields)]
struct ItemProgressWire {
    /// See [`ItemProgress`].
    id: ItemId,
    /// See [`ItemProgress`].
    phase: ItemPhase,
    /// See [`ItemProgress`]; an item written without fix passes has none.
    #[serde(default)]
    fix_passes: Vec<FixPass>,
    /// See [`ItemProgress`]; absent until the gate has run.
    #[serde(default)]
    gate: Option<GateResult>,
}

checked!(ItemProgress, ItemProgressWire, check_item_progress);

/// An item's progress inside a team, to change through its operations only.
///
/// It reads like an [`ItemProgress`] (it dereferences to one) but is not a `&mut ItemProgress`: the progress cannot be
/// overwritten with another, so the phase changes only through [`ItemMut::advance`], which keeps to the table of
/// phase changes and checks the reviews.
pub type ItemMut<'a> = CheckedMut<'a, ItemProgress>;

impl CheckedMut<'_, ItemProgress> {
    /// See [`ItemProgress::advance`]. An item in review is approved only with the pull request and the reviews it has:
    /// the outcomes reach it through [`CheckedMut::submit_review`], not through the phase handed in. The pull request
    /// that was reviewed is also the one the item is fixed on and merged with.
    ///
    /// # Errors
    /// [`AdvanceError`], as for [`ItemProgress::advance`]; [`AdvanceError::RoundNotKept`] when an approval carries
    /// another pull request, other reviews, or an outcome that was not submitted, or when a fix pass or a merge
    /// carries another pull request than the reviewed one.
    pub fn advance(&mut self, next: ItemPhase) -> Result<(), AdvanceError> {
        self.value.check_round_kept(&next)?;
        self.value.advance(next)
    }

    /// See [`ItemProgress::reassign`].
    ///
    /// # Errors
    /// [`NotWorkedOn`] when the item is in any other phase than in progress or fixing.
    pub fn reassign(&mut self, assignee: AgentName, branch: BranchName) -> Result<(), NotWorkedOn> {
        self.value.reassign(assignee, branch)
    }

    /// See [`ItemProgress::submit_review`].
    ///
    /// # Errors
    /// [`SubmitReviewError`], as for [`ItemProgress::submit_review`].
    pub fn submit_review(
        &mut self,
        task: &TaskId,
        verdict: ReviewVerdict,
        at: Timestamp,
    ) -> Result<(), SubmitReviewError> {
        self.value.submit_review(task, verdict, at)
    }

    /// Records the last gate result for the item, replacing the one before.
    pub fn set_gate(&mut self, result: GateResult) {
        self.value.gate = Some(result);
    }
}

/// A fix pass that no review asked for, found when reading an item.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("item {item}: fix pass {pass} is not valid: {source}")]
struct InvalidFixPass {
    /// The item.
    item: ItemId,
    /// The fix pass, counted from 1.
    pass: usize,
    /// What is wrong with it.
    #[source]
    source: InvalidReviews,
}

/// Either independently typed invariant violation found while reading an item's progress.
#[derive(Debug, thiserror::Error)]
enum InvalidItemProgress {
    /// A fixing item has no pass.
    #[error(transparent)]
    Fixing(#[from] FixingWithoutFixPass),
    /// A pass has invalid reviews.
    #[error(transparent)]
    Reviews(#[from] InvalidFixPass),
}

/// An item that is fixing but has no fix pass behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("item {item} is fixing but records no fix pass")]
pub struct FixingWithoutFixPass {
    /// The item.
    pub item: ItemId,
}

/// A phase change the table does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("item {item} cannot change from {from} to {to}")]
pub struct InvalidAdvance {
    /// The item.
    pub item: ItemId,
    /// Its phase.
    pub from: PhaseKind,
    /// The refused phase.
    pub to: PhaseKind,
}

/// Reviews that do not fit the phase or the fix pass that carries them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidReviews {
    /// An approved item carries no review: nothing approved it.
    #[error("an approved item must carry the reviews that approved it, and has none")]
    ApprovedWithoutReviews,
    /// An approved item carries a review that is not an approval.
    #[error("the review by {reviewer} is not an approval: an approved item carries only approving reviews")]
    ApprovedNotByAll {
        /// The reviewer whose review is pending or asks for changes.
        reviewer: InvestigatorName,
    },
    /// One reviewer has two reviews in one round.
    #[error("the reviewer {reviewer} has two reviews in one round")]
    DuplicateReviewer {
        /// The repeated reviewer.
        reviewer: InvestigatorName,
    },
    /// One reviewer task has two reviews in one round.
    #[error("the task {task} has two reviews in one round")]
    DuplicateTask {
        /// The repeated task.
        task: TaskId,
    },
    /// A fix pass records no review: nothing sent the item back.
    #[error("a fix pass must record the reviews that sent the item back, and has none")]
    FixPassWithoutReviews,
    /// Every review of the round approved: the item is approved, not sent back.
    #[error("every review of the round approved: the item cannot go back for a fix pass")]
    FixPassApproved,
    /// No reviewer submitted `REQUEST_CHANGES`: the item was not sent back.
    #[error("a fix pass must record at least one submitted REQUEST_CHANGES review")]
    FixPassNotRequested,
}

/// Why an item could not move to another phase.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdvanceError {
    /// [`PhaseKind::successors`] does not allow the change.
    #[error(transparent)]
    Transition(#[from] InvalidAdvance),
    /// The reviews of the round do not fit the phase.
    #[error("item {item}: {source}")]
    Reviews {
        /// The item.
        item: ItemId,
        /// What is wrong with the reviews.
        #[source]
        source: InvalidReviews,
    },
    /// The change does not carry the round the item is in: an approval is not made of the pull request and the reviews
    /// as recorded, or a fix pass or a merge is not on the reviewed pull request.
    #[error(
        "item {item}: an approval carries the item's own pull request and reviews, and a fix pass or a merge its reviewed pull request, as recorded"
    )]
    RoundNotKept {
        /// The item.
        item: ItemId,
    },
    /// `Completion::Listed` is for an item the task list already marked done: a team finishes an item by its merge.
    #[error("item {item} cannot be completed as listed: only an item the task list marks done starts that way")]
    ListedCompletion {
        /// The item.
        item: ItemId,
    },
}

/// Why the outcome of a review could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubmitReviewError {
    /// Only an item in review has reviews that wait for their outcome.
    #[error("item {item} is {phase}: only an item in review receives the outcome of a review")]
    NotInReview {
        /// The item.
        item: ItemId,
        /// Its phase.
        phase: PhaseKind,
    },
    /// The round asked no review of this task.
    #[error("item {item}: no review of the round was asked of task {task}")]
    UnknownReview {
        /// The item.
        item: ItemId,
        /// The task that is not a reviewer of the round.
        task: TaskId,
    },
    /// The review has its outcome already.
    #[error("item {item}: the review by task {task} has its outcome already")]
    AlreadySubmitted {
        /// The item.
        item: ItemId,
        /// The reviewer's task.
        task: TaskId,
    },
}

/// An [`AdvanceError`] equals the refused change it wraps, so that a caller can compare it with the edge it expects.
impl PartialEq<InvalidAdvance> for AdvanceError {
    fn eq(&self, other: &InvalidAdvance) -> bool {
        matches!(self, Self::Transition(refused) if refused == other)
    }
}

/// The item is not being worked on: it has no assignee to replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("item {item} is {phase}: only an item in progress or fixing has an assignee")]
pub struct NotWorkedOn {
    /// The item.
    pub item: ItemId,
    /// Its phase.
    pub phase: PhaseKind,
}

/// Checks that a fixing item has a fix pass behind it.
fn check_has_fix_pass(progress: &ItemProgress) -> Result<(), FixingWithoutFixPass> {
    if progress.phase.kind() == PhaseKind::Fixing && progress.fix_passes.is_empty() {
        return Err(FixingWithoutFixPass { item: progress.id });
    }
    Ok(())
}

/// Checks the fixing phase and the reviews of every fix pass, with the pass's number in the latter error.
fn check_item_progress(progress: &ItemProgress) -> Result<(), InvalidItemProgress> {
    check_has_fix_pass(progress)?;
    for (index, pass) in progress.fix_passes.iter().enumerate() {
        check_fix_pass(&pass.requested_by).map_err(|source| InvalidFixPass {
            item: progress.id,
            pass: index.saturating_add(1),
            source,
        })?;
    }
    Ok(())
}

impl ItemProgress {
    /// The progress of an item that is not started, or of one that the task list already marks done.
    #[must_use]
    pub fn new(id: ItemId, mark: ItemMark) -> Self {
        let phase = match mark {
            ItemMark::Open => ItemPhase::Pending,
            ItemMark::Done => ItemPhase::Done {
                completion: Completion::Listed,
            },
        };
        Self {
            id,
            phase,
            fix_passes: Vec::new(),
            gate: None,
        }
    }

    getter!("The item's number in the task list.", pub const fn id(&self) -> ItemId = self.id);

    getter!("Where the item is.", pub const fn phase(&self) -> &ItemPhase = &self.phase);

    getter!("The item status the phase shows.", pub const fn status(&self) -> ItemStatus = self.phase.kind().status());

    getter!("The fix passes so far, oldest first.", pub fn fix_passes(&self) -> &[FixPass] = &self.fix_passes);

    /// Moves the item to `next`. Leaving `in_review` for `fixing` records the round's reviews as a fix pass.
    ///
    /// # Errors
    /// [`AdvanceError::Transition`] when [`PhaseKind::successors`] does not allow the change;
    /// [`AdvanceError::Reviews`] when the reviews of the new phase do not fit it ([`ItemPhase::check_reviews`]), or
    /// when no review asked for the fix pass the change would record; [`AdvanceError::ListedCompletion`] when
    /// `next` completes the item as [`Completion::Listed`], which only [`ItemProgress::new`] gives.
    pub fn advance(&mut self, next: ItemPhase) -> Result<(), AdvanceError> {
        let (from, to) = (self.phase.kind(), next.kind());
        if !from.successors().contains(&to) {
            return Err(InvalidAdvance {
                item: self.id,
                from,
                to,
            }
            .into());
        }
        if matches!(
            next,
            ItemPhase::Done {
                completion: Completion::Listed
            }
        ) {
            return Err(AdvanceError::ListedCompletion { item: self.id });
        }
        next.check_reviews()
            .map_err(|source| AdvanceError::Reviews { item: self.id, source })?;
        if let (ItemPhase::InReview { reviews, .. }, PhaseKind::Fixing) = (&self.phase, to) {
            check_fix_pass(reviews).map_err(|source| AdvanceError::Reviews { item: self.id, source })?;
        }
        let left = std::mem::replace(&mut self.phase, next);
        if let (ItemPhase::InReview { reviews, .. }, PhaseKind::Fixing) = (left, to) {
            self.fix_passes.push(FixPass { requested_by: reviews });
        }
        Ok(())
    }

    /// Checks that the round an item has is kept: an approval carries exactly its pull request and reviews, and the
    /// pull request that was reviewed is the one the item goes back to be fixed on and is merged with. Any other
    /// change is left to [`ItemProgress::advance`].
    ///
    /// # Errors
    /// [`AdvanceError::RoundNotKept`] when `next` carries another round than the one recorded.
    fn check_round_kept(&self, next: &ItemPhase) -> Result<(), AdvanceError> {
        let kept = match (&self.phase, next) {
            (
                ItemPhase::InReview {
                    pr: kept_pr,
                    reviews: kept,
                },
                ItemPhase::Approved { pr, reviews },
            ) => kept_pr == pr && kept == reviews,
            (ItemPhase::InReview { pr: kept_pr, .. }, ItemPhase::Fixing { pr, .. })
            | (
                ItemPhase::Approved { pr: kept_pr, .. },
                ItemPhase::Done {
                    completion: Completion::Merged { pr, .. },
                },
            ) => kept_pr == pr,
            _ => true,
        };
        if kept {
            return Ok(());
        }
        Err(AdvanceError::RoundNotKept { item: self.id })
    }

    /// Records the verdict that the reviewer task `task` reported for the item's pull request.
    ///
    /// # Errors
    /// [`SubmitReviewError`]: the item is not in review, no review of the round was asked of `task`, or that review
    /// already has its outcome.
    pub fn submit_review(
        &mut self,
        task: &TaskId,
        verdict: ReviewVerdict,
        at: Timestamp,
    ) -> Result<(), SubmitReviewError> {
        let item = self.id;
        let phase = self.phase.kind();
        let ItemPhase::InReview { reviews, .. } = &mut self.phase else {
            return Err(SubmitReviewError::NotInReview { item, phase });
        };
        let Some(review) = reviews.iter_mut().find(|review| &review.task == task) else {
            return Err(SubmitReviewError::UnknownReview {
                item,
                task: task.clone(),
            });
        };
        match review.outcome {
            ReviewOutcome::Pending => {
                review.outcome = ReviewOutcome::Submitted { verdict, at };
                Ok(())
            }
            ReviewOutcome::Submitted { .. } => Err(SubmitReviewError::AlreadySubmitted {
                item,
                task: task.clone(),
            }),
        }
    }

    /// Gives an item that is in progress or fixing to another member.
    ///
    /// # Errors
    /// [`NotWorkedOn`] when the item is in any other phase.
    pub fn reassign(&mut self, assignee: AgentName, branch: BranchName) -> Result<(), NotWorkedOn> {
        match &mut self.phase {
            ItemPhase::InProgress {
                assignee: current,
                branch: current_branch,
            }
            | ItemPhase::Fixing {
                assignee: current,
                branch: current_branch,
                ..
            } => {
                *current = assignee;
                *current_branch = branch;
                Ok(())
            }
            ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => {
                Err(NotWorkedOn {
                    item: self.id,
                    phase: self.phase.kind(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::kinds::tests::{assert_keywords, assert_refuses};
    use crate::state::GateOutcome;
    use crate::state::fixtures::{agent, assert_refused, at, branch, pr, review};
    use crate::status::StatusMachine;

    const PHASES: &[(PhaseKind, &str)] = &[
        (PhaseKind::Pending, "pending"),
        (PhaseKind::InProgress, "in_progress"),
        (PhaseKind::InReview, "in_review"),
        (PhaseKind::Fixing, "fixing"),
        (PhaseKind::Approved, "approved"),
        (PhaseKind::Done, "done"),
    ];

    /// An item's serialized shape, used to construct states that the public constructors refuse.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ItemProgressWire {
        /// See [`ItemProgress`].
        id: ItemId,
        /// See [`ItemProgress`].
        phase: ItemPhase,
        /// See [`ItemProgress`].
        #[serde(default)]
        fix_passes: Vec<FixPass>,
        /// See [`ItemProgress`].
        #[serde(default)]
        gate: Option<GateResult>,
    }

    impl TryFrom<ItemProgressWire> for ItemProgress {
        type Error = FixingWithoutFixPass;

        fn try_from(wire: ItemProgressWire) -> Result<Self, Self::Error> {
            let progress = Self {
                id: wire.id,
                phase: wire.phase,
                fix_passes: wire.fix_passes,
                gate: wire.gate,
            };
            check_has_fix_pass(&progress)?;
            Ok(progress)
        }
    }

    /// The id of the item of these tests.
    fn id() -> ItemId {
        "1.2".parse().unwrap()
    }

    /// The two reviews of a round, both asked for.
    fn reviews(outcome: ReviewOutcome) -> Vec<Review> {
        vec![review("inv-opus", 5, outcome), review("inv-codex", 6, outcome)]
    }

    /// A phase of kind `kind`, with data.
    fn phase_of(kind: PhaseKind) -> ItemPhase {
        match kind {
            PhaseKind::Pending => ItemPhase::Pending,
            PhaseKind::InProgress => ItemPhase::InProgress {
                assignee: agent("codex-1"),
                branch: branch("agents/codex-1/item-1-2"),
            },
            PhaseKind::InReview => ItemPhase::InReview {
                pr: pr(7),
                reviews: reviews(ReviewOutcome::Pending),
            },
            PhaseKind::Fixing => ItemPhase::Fixing {
                assignee: agent("codex-1"),
                branch: branch("agents/codex-1/item-1-2"),
                pr: pr(7),
            },
            PhaseKind::Approved => ItemPhase::Approved {
                pr: pr(7),
                reviews: reviews(ReviewOutcome::Submitted {
                    verdict: ReviewVerdict::Approve,
                    at: at(40),
                }),
            },
            PhaseKind::Done => ItemPhase::Done {
                completion: Completion::Merged { pr: pr(7), at: at(50) },
            },
        }
    }

    /// An item that has walked to phase `kind` the shortest way.
    fn progress_in(kind: PhaseKind) -> ItemProgress {
        let way: &[PhaseKind] = match kind {
            PhaseKind::Pending => &[],
            PhaseKind::InProgress => &[PhaseKind::InProgress],
            PhaseKind::InReview => &[PhaseKind::InProgress, PhaseKind::InReview],
            PhaseKind::Fixing => &[PhaseKind::InProgress, PhaseKind::InReview, PhaseKind::Fixing],
            PhaseKind::Approved => &[PhaseKind::InProgress, PhaseKind::InReview, PhaseKind::Approved],
            PhaseKind::Done => &[
                PhaseKind::InProgress,
                PhaseKind::InReview,
                PhaseKind::Approved,
                PhaseKind::Done,
            ],
        };
        let mut progress = ItemProgress::new(id(), ItemMark::Open);
        for step in way {
            let next = match step {
                PhaseKind::InReview if way.contains(&PhaseKind::Fixing) => ItemPhase::InReview {
                    pr: pr(7),
                    reviews: reviews(ReviewOutcome::Submitted {
                        verdict: ReviewVerdict::RequestChanges,
                        at: at(30),
                    }),
                },
                _ => phase_of(*step),
            };
            progress.advance(next).unwrap();
        }
        progress
    }

    // ---- the phases ---------------------------------------------------------------------------------------------

    #[test]
    fn phases_have_their_texts_and_round_trip() {
        assert_eq!(PhaseKind::ALL, PHASES.iter().map(|&(k, _)| k).collect::<Vec<_>>());
        assert_keywords(PHASES);
        assert_refuses("item phase", PHASES, &["review", "merged", "in-progress"]);
    }

    #[test]
    fn the_table_of_phase_changes_is_as_documented() {
        let table: &[(PhaseKind, &[PhaseKind])] = &[
            (PhaseKind::Pending, &[PhaseKind::InProgress]),
            (PhaseKind::InProgress, &[PhaseKind::Pending, PhaseKind::InReview]),
            (PhaseKind::InReview, &[PhaseKind::Fixing, PhaseKind::Approved]),
            (PhaseKind::Fixing, &[PhaseKind::InReview]),
            (PhaseKind::Approved, &[PhaseKind::Done]),
            (PhaseKind::Done, &[]),
        ];
        for &(from, to) in table {
            assert_eq!(from.successors(), to, "{from}");
        }
    }

    #[test]
    fn every_phase_change_is_a_status_change_the_item_status_allows() {
        for from in PhaseKind::ALL {
            for to in from.successors() {
                from.status()
                    .transition_to(to.status())
                    .unwrap_or_else(|err| panic!("{from} -> {to}: {err}"));
            }
        }
    }

    #[test]
    fn every_status_change_the_item_status_allows_has_a_phase_change() {
        for from in ItemStatus::ALL {
            for to in from.successors() {
                let covered = PhaseKind::ALL
                    .iter()
                    .any(|k| k.status() == *from && k.successors().iter().any(|next| next.status() == *to));
                assert!(covered, "{from} -> {to}");
            }
        }
    }

    #[test]
    fn a_phase_knows_its_kind_and_its_text_in_json() {
        for &(kind, text) in PHASES {
            let phase = phase_of(kind);

            assert_eq!(phase.kind(), kind);
            assert_eq!(serde_json::to_value(&phase).unwrap()["phase"], json!(text));
        }
    }

    #[test]
    fn the_status_of_a_phase_is_the_status_of_its_kind() {
        let expected = [
            (PhaseKind::Pending, ItemStatus::Pending),
            (PhaseKind::InProgress, ItemStatus::InProgress),
            (PhaseKind::Fixing, ItemStatus::InProgress),
            (PhaseKind::InReview, ItemStatus::InReview),
            (PhaseKind::Approved, ItemStatus::Approved),
            (PhaseKind::Done, ItemStatus::Done),
        ];
        for (kind, status) in expected {
            assert_eq!(kind.status(), status, "{kind}");
            assert_eq!(progress_in(kind).status(), status, "{kind}");
        }
    }

    // ---- an item's progress -------------------------------------------------------------------------------------

    #[test]
    fn an_open_item_starts_pending_and_a_listed_one_starts_done() {
        let open = ItemProgress::new(id(), ItemMark::Open);
        let done = ItemProgress::new(id(), ItemMark::Done);

        assert_eq!((open.id(), open.phase()), (id(), &ItemPhase::Pending));
        assert_eq!(
            done.phase(),
            &ItemPhase::Done {
                completion: Completion::Listed
            }
        );
        assert_eq!(done.status(), ItemStatus::Done);
        assert!(open.fix_passes().is_empty() && open.gate.is_none());
    }

    #[test]
    fn an_item_walks_from_pending_to_done_and_a_fix_pass_is_recorded_on_the_way() {
        let mut item = ItemProgress::new(id(), ItemMark::Open);
        let sent_back = reviews(ReviewOutcome::Submitted {
            verdict: ReviewVerdict::RequestChanges,
            at: at(30),
        });

        item.advance(phase_of(PhaseKind::InProgress)).unwrap();
        item.advance(ItemPhase::InReview {
            pr: pr(7),
            reviews: sent_back.clone(),
        })
        .unwrap();
        item.advance(phase_of(PhaseKind::Fixing)).unwrap();
        assert_eq!(
            item.fix_passes(),
            [FixPass {
                requested_by: sent_back
            }]
        );
        item.advance(phase_of(PhaseKind::InReview)).unwrap();
        item.advance(phase_of(PhaseKind::Approved)).unwrap();
        item.advance(phase_of(PhaseKind::Done)).unwrap();

        assert_eq!(item.phase(), &phase_of(PhaseKind::Done));
        assert_eq!(item.fix_passes().len(), 1);
    }

    #[test]
    fn each_round_that_asks_for_changes_is_a_fix_pass_in_order() {
        let requested = ReviewOutcome::Submitted {
            verdict: ReviewVerdict::RequestChanges,
            at: at(30),
        };
        let mut item = progress_in(PhaseKind::Fixing);
        let second = vec![review("inv-opus", 8, requested)];

        item.advance(ItemPhase::InReview {
            pr: pr(7),
            reviews: second.clone(),
        })
        .unwrap();
        item.advance(phase_of(PhaseKind::Fixing)).unwrap();

        assert_eq!(item.fix_passes().len(), 2);
        assert_eq!(item.fix_passes().last().map(|p| p.requested_by.clone()), Some(second));
        assert_eq!(item.fix_passes()[0].requested_by, reviews(requested));
    }

    #[test]
    fn leaving_review_for_approval_is_no_fix_pass() {
        let item = progress_in(PhaseKind::Approved);

        assert_eq!(item.fix_passes(), []);
    }

    #[test]
    fn a_member_who_leaves_puts_the_item_back_to_pending() {
        let mut item = progress_in(PhaseKind::InProgress);

        item.advance(ItemPhase::Pending).unwrap();

        assert_eq!(item.phase(), &ItemPhase::Pending);
    }

    #[test]
    fn a_change_the_table_does_not_allow_is_refused_and_changes_nothing() {
        for from in PhaseKind::ALL {
            for to in PhaseKind::ALL {
                if from.successors().contains(to) {
                    continue;
                }
                let mut item = progress_in(*from);
                let before = item.clone();

                let err = item.advance(phase_of(*to)).unwrap_err();

                assert_eq!(
                    err,
                    InvalidAdvance {
                        item: id(),
                        from: *from,
                        to: *to
                    },
                    "{from} -> {to}"
                );
                assert_eq!(item, before, "{from} -> {to}");
            }
        }
    }

    #[test]
    fn an_item_in_progress_or_fixing_is_given_to_another_member() {
        for kind in [PhaseKind::InProgress, PhaseKind::Fixing] {
            let mut item = progress_in(kind);

            item.reassign(agent("claude-1"), branch("agents/claude-1/item-1-2"))
                .unwrap();

            let (ItemPhase::InProgress {
                assignee,
                branch: new_branch,
                ..
            }
            | ItemPhase::Fixing {
                assignee,
                branch: new_branch,
                ..
            }) = item.phase()
            else {
                panic!("{kind}: the phase changed");
            };
            assert_eq!(
                (assignee.as_str(), new_branch.as_str()),
                ("claude-1", "agents/claude-1/item-1-2")
            );
            assert_eq!(item.phase().kind(), kind);
        }
    }

    #[test]
    fn reassigning_keeps_the_pull_request_of_a_fix_pass() {
        let mut item = progress_in(PhaseKind::Fixing);

        item.reassign(agent("claude-1"), branch("agents/claude-1/item-1-2"))
            .unwrap();

        let ItemPhase::Fixing { pr: kept, .. } = item.phase() else {
            panic!("not fixing");
        };
        assert_eq!(kept, &pr(7));
        assert_eq!(item.fix_passes().len(), 1);
    }

    #[test]
    fn an_item_in_any_other_phase_has_no_assignee_to_replace() {
        for kind in [
            PhaseKind::Pending,
            PhaseKind::InReview,
            PhaseKind::Approved,
            PhaseKind::Done,
        ] {
            let mut item = progress_in(kind);
            let before = item.clone();

            let err = item.reassign(agent("claude-1"), branch("b")).unwrap_err();

            assert_eq!(
                err,
                NotWorkedOn {
                    item: id(),
                    phase: kind
                }
            );
            assert_eq!(item, before);
        }
    }

    // ---- json ---------------------------------------------------------------------------------------------------

    #[test]
    fn every_phase_round_trips_through_json() {
        for &(kind, _) in PHASES {
            let phase = phase_of(kind);

            let value = serde_json::to_value(&phase).unwrap();

            assert_eq!(serde_json::from_value::<ItemPhase>(value).unwrap(), phase, "{kind}");
        }
        let listed = ItemPhase::Done {
            completion: Completion::Listed,
        };
        assert_eq!(
            serde_json::to_value(&listed).unwrap(),
            json!({"phase": "done", "completion": {"how": "listed"}})
        );
        assert_eq!(
            serde_json::from_value::<ItemPhase>(serde_json::to_value(&listed).unwrap()).unwrap(),
            listed
        );
    }

    #[test]
    fn an_item_in_review_is_written_with_its_pull_request_and_reviews() {
        let item = progress_in(PhaseKind::InReview);

        let value = serde_json::to_value(&item).unwrap();

        assert_eq!(
            value,
            json!({
                "id": "1.2",
                "phase": {
                    "phase": "in_review",
                    "pr": {"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:15:10Z"},
                    "reviews": [
                        {"reviewer": "inv-opus", "task": "task-20261111-153005-k3x9", "requested_at": "2026-11-11T19:15:20Z", "outcome": {"status": "pending"}},
                        {"reviewer": "inv-codex", "task": "task-20261111-153006-k3x9", "requested_at": "2026-11-11T19:15:20Z", "outcome": {"status": "pending"}}
                    ]
                }
            })
        );
        assert_eq!(serde_json::from_value::<ItemProgress>(value).unwrap(), item);
    }

    #[test]
    fn a_submitted_review_is_written_with_its_verdict_and_time() {
        let review = review(
            "inv-opus",
            5,
            ReviewOutcome::Submitted {
                verdict: ReviewVerdict::RequestChanges,
                at: at(30),
            },
        );

        let value = serde_json::to_value(&review).unwrap();

        assert_eq!(
            value["outcome"],
            json!({"status": "submitted", "verdict": "REQUEST_CHANGES", "at": "2026-11-11T19:15:30Z"})
        );
        assert_eq!(serde_json::from_value::<Review>(value).unwrap(), review);
    }

    #[test]
    fn fix_passes_and_the_gate_result_are_written_when_there_are_some_and_read_back() {
        let mut item = progress_in(PhaseKind::Fixing);
        item.gate = Some(GateResult {
            head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
            finished_at: at(45),
            outcome: GateOutcome::Failed {
                exit_code: std::num::NonZeroI32::new(2).unwrap(),
            },
            log: "/home/u/agents/tasks/task-1/gate.log".parse().unwrap(),
        });

        let value = serde_json::to_value(&item).unwrap();

        assert_eq!(value["fix_passes"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["gate"]["outcome"], json!({"kind": "failed", "exit_code": 2}));
        assert_eq!(serde_json::from_value::<ItemProgress>(value).unwrap(), item);
        let quiet = serde_json::to_value(progress_in(PhaseKind::Pending)).unwrap();
        assert_eq!(quiet, json!({"id": "1.2", "phase": {"phase": "pending"}}));
    }

    #[test]
    fn an_item_fixing_without_a_fix_pass_is_refused_when_read() {
        let value = json!({
            "id": "1.2",
            "phase": {
                "phase": "fixing",
                "assignee": "codex-1",
                "branch": "agents/codex-1/item-1-2",
                "pr": {"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:15:10Z"}
            }
        });

        let wire: ItemProgressWire = serde_json::from_value(value).unwrap();

        assert_eq!(ItemProgress::try_from(wire), Err(FixingWithoutFixPass { item: id() }));
    }

    #[test]
    fn an_item_with_an_unknown_phase_field_or_status_is_refused() {
        let bad = [
            json!({"id": "1.2", "phase": {"phase": "reviewing"}}),
            json!({"id": "1.2", "phase": {"phase": "pending"}, "extra": 1}),
            json!({"id": "1.2", "phase": {"phase": "in_progress", "assignee": "codex-1"}}),
            json!({"id": "0.2", "phase": {"phase": "pending"}}),
            json!({"id": "1.2", "phase": {"phase": "done", "completion": {"how": "wished"}}}),
        ];
        for value in bad {
            assert_refused::<ItemProgress>(&value.clone());
        }
    }

    // ---- the reviews of a round ---------------------------------------------------------------------------------

    /// A submitted review of `reviewer` (task `n`) with `verdict`.
    fn submitted(reviewer: &str, n: u32, verdict: ReviewVerdict) -> Review {
        review(reviewer, n, ReviewOutcome::Submitted { verdict, at: at(40) })
    }

    /// An item in review with `round`.
    fn in_review_with(round: Vec<Review>) -> ItemProgress {
        let mut item = progress_in(PhaseKind::InProgress);
        item.advance(ItemPhase::InReview {
            pr: pr(7),
            reviews: round,
        })
        .unwrap();
        item
    }

    /// What `advance` says to the item in review `from` moving to approval with `round`, and the item after it.
    fn approve_with(round: Vec<Review>) -> (Result<(), AdvanceError>, ItemProgress, ItemProgress) {
        let mut item = progress_in(PhaseKind::InReview);
        let before = item.clone();
        let result = item.advance(ItemPhase::Approved {
            pr: pr(7),
            reviews: round,
        });
        (result, before, item)
    }

    #[test]
    fn an_item_is_approved_only_with_reviews_that_all_approved() {
        let cases = [
            (vec![], InvalidReviews::ApprovedWithoutReviews),
            (
                reviews(ReviewOutcome::Pending),
                InvalidReviews::ApprovedNotByAll {
                    reviewer: "inv-opus".parse().unwrap(),
                },
            ),
            (
                vec![
                    submitted("inv-opus", 5, ReviewVerdict::Approve),
                    submitted("inv-codex", 6, ReviewVerdict::RequestChanges),
                ],
                InvalidReviews::ApprovedNotByAll {
                    reviewer: "inv-codex".parse().unwrap(),
                },
            ),
        ];
        for (round, cause) in cases {
            let (result, before, after) = approve_with(round);

            assert_eq!(
                result,
                Err(AdvanceError::Reviews {
                    item: id(),
                    source: cause
                })
            );
            assert_eq!(after, before);
        }
    }

    #[test]
    fn an_item_whose_reviews_all_approved_is_approved() {
        let (result, _, after) = approve_with(vec![
            submitted("inv-opus", 5, ReviewVerdict::Approve),
            submitted("inv-codex", 6, ReviewVerdict::Approve),
        ]);

        assert_eq!(result, Ok(()));
        assert_eq!(after.status(), ItemStatus::Approved);
    }

    #[test]
    fn a_reviewer_or_a_task_twice_in_one_round_is_refused() {
        let cases = [
            (
                vec![
                    review("inv-opus", 5, ReviewOutcome::Pending),
                    review("inv-opus", 6, ReviewOutcome::Pending),
                ],
                InvalidReviews::DuplicateReviewer {
                    reviewer: "inv-opus".parse().unwrap(),
                },
            ),
            (
                vec![
                    review("inv-opus", 5, ReviewOutcome::Pending),
                    review("inv-codex", 5, ReviewOutcome::Pending),
                ],
                InvalidReviews::DuplicateTask {
                    task: crate::state::fixtures::task(5),
                },
            ),
        ];
        for (round, cause) in cases {
            let mut item = progress_in(PhaseKind::InProgress);
            let before = item.clone();

            let result = item.advance(ItemPhase::InReview {
                pr: pr(7),
                reviews: round,
            });

            assert_eq!(
                result,
                Err(AdvanceError::Reviews {
                    item: id(),
                    source: cause
                })
            );
            assert_eq!(item, before);
        }
    }

    #[test]
    fn a_round_that_asked_for_changes_or_is_not_complete_sends_the_item_back() {
        for round in [
            vec![
                submitted("inv-opus", 5, ReviewVerdict::Approve),
                submitted("inv-codex", 6, ReviewVerdict::RequestChanges),
            ],
            vec![
                submitted("inv-opus", 5, ReviewVerdict::RequestChanges),
                review("inv-codex", 6, ReviewOutcome::Pending),
            ],
        ] {
            let mut item = in_review_with(round);

            assert_eq!(item.advance(phase_of(PhaseKind::Fixing)), Ok(()));
            assert_eq!(item.fix_passes().len(), 1);
        }
    }

    #[test]
    fn an_item_cannot_go_back_for_a_fix_pass_that_no_review_asked_for() {
        let cases = [
            (vec![], InvalidReviews::FixPassWithoutReviews),
            (
                vec![
                    submitted("inv-opus", 5, ReviewVerdict::Approve),
                    submitted("inv-codex", 6, ReviewVerdict::Approve),
                ],
                InvalidReviews::FixPassApproved,
            ),
            (reviews(ReviewOutcome::Pending), InvalidReviews::FixPassNotRequested),
            (
                vec![
                    submitted("inv-opus", 5, ReviewVerdict::Approve),
                    review("inv-codex", 6, ReviewOutcome::Pending),
                ],
                InvalidReviews::FixPassNotRequested,
            ),
        ];
        for (round, cause) in cases {
            let mut item = in_review_with(round);
            let before = item.clone();

            let result = item.advance(phase_of(PhaseKind::Fixing));

            assert_eq!(
                result,
                Err(AdvanceError::Reviews {
                    item: id(),
                    source: cause
                })
            );
            assert_eq!(item, before);
        }
    }

    #[test]
    fn an_advance_error_equals_the_refused_change_it_wraps_and_nothing_else() {
        let refused = InvalidAdvance {
            item: id(),
            from: PhaseKind::Pending,
            to: PhaseKind::Done,
        };
        let error = AdvanceError::from(refused);

        assert_eq!(error, refused);
        assert_ne!(
            AdvanceError::Reviews {
                item: id(),
                source: InvalidReviews::ApprovedWithoutReviews
            },
            refused
        );
    }

    /// The JSON of an approved phase with `reviews`.
    fn approved_json(reviews: &serde_json::Value) -> serde_json::Value {
        json!({
            "phase": "approved",
            "pr": {"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:15:10Z"},
            "reviews": reviews
        })
    }

    /// The JSON of a review of `reviewer` (task `n`) with `outcome`.
    fn review_json(reviewer: &str, n: u32, outcome: &serde_json::Value) -> serde_json::Value {
        json!({"reviewer": reviewer, "task": format!("task-20261111-1530{n:02}-k3x9"), "requested_at": "2026-11-11T19:15:20Z", "outcome": outcome})
    }

    #[test]
    fn an_approved_phase_whose_reviews_did_not_all_approve_is_refused_when_read() {
        let approve = json!({"status": "submitted", "verdict": "APPROVE", "at": "2026-11-11T19:15:30Z"});
        let changes = json!({"status": "submitted", "verdict": "REQUEST_CHANGES", "at": "2026-11-11T19:15:30Z"});
        let pending = json!({"status": "pending"});
        assert_refused::<ItemPhase>(&approved_json(&json!([])));
        assert_refused::<ItemPhase>(&approved_json(&json!([review_json("inv-opus", 5, &pending)])));
        assert_refused::<ItemPhase>(&approved_json(&json!([
            review_json("inv-opus", 5, &approve),
            review_json("inv-codex", 6, &changes)
        ])));
        serde_json::from_value::<ItemPhase>(approved_json(&json!([
            review_json("inv-opus", 5, &approve),
            review_json("inv-codex", 6, &approve)
        ])))
        .unwrap();
    }

    #[test]
    fn a_round_with_a_reviewer_twice_is_refused_when_read() {
        let pending = json!({"status": "pending"});
        let value = json!({
            "phase": "in_review",
            "pr": {"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:15:10Z"},
            "reviews": [review_json("inv-opus", 5, &pending), review_json("inv-opus", 6, &pending)]
        });

        assert_refused::<ItemPhase>(&value);
    }

    #[test]
    fn a_fix_pass_without_a_request_for_changes_is_refused_when_read() {
        let approve = json!({"status": "submitted", "verdict": "APPROVE", "at": "2026-11-11T19:15:30Z"});
        let changes = json!({"status": "submitted", "verdict": "REQUEST_CHANGES", "at": "2026-11-11T19:15:30Z"});
        let pending = json!({"status": "pending"});
        let fixing = |requested_by: &serde_json::Value| {
            json!({
                "id": "1.2",
                "phase": {
                    "phase": "fixing",
                    "assignee": "codex-1",
                    "branch": "agents/codex-1/item-1-2",
                    "pr": {"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:15:10Z"}
                },
                "fix_passes": [{"requested_by": requested_by}]
            })
        };

        assert_refused::<ItemProgress>(&fixing(&json!([])));
        assert_refused::<ItemProgress>(&fixing(&json!([review_json("inv-opus", 5, &approve)])));
        assert_refused::<ItemProgress>(&fixing(&json!([review_json("inv-opus", 5, &pending)])));
        assert_refused::<ItemProgress>(&fixing(&json!([
            review_json("inv-opus", 5, &changes),
            review_json("inv-opus", 6, &changes)
        ])));
        serde_json::from_value::<ItemProgress>(fixing(&json!([review_json("inv-opus", 5, &changes)]))).unwrap();
    }
}
