//! The progress of a task-list item: who works on it, its pull request, its reviews and fix passes, and its gate
//! result.

use std::collections::HashSet;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::checked::CheckedMut;
use super::gate::GateResult;
use super::strict::strict_enum;
use super::values::{PrNumber, Timestamp};
use super::{CommitId, Findings};
use crate::ids::{AgentName, BranchName, InvestigatorName, ItemId, TaskId};
use crate::kinds::{Keyword, UnknownKeyword};
use crate::status::{ItemStatus, ReviewVerdict};
use crate::task_list::ItemMark;

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
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<PhaseKind>)]
pub enum PhaseKind {
    /// Not started.
    Pending,
    /// A member works on it, or the coordinator integrates it, until its pull request is open.
    InProgress,
    /// Its pull request is open and waits for its two reviews.
    InReview,
    /// The reviewers asked for changes: the team fixes them (status `in_progress`).
    Fixing,
    /// Both reviews approve; it waits for its merge.
    Approved,
    /// Merged, or listed as done.
    Done,
}

impl Keyword for PhaseKind {
    const WHAT: &'static str = "item phase";
}

impl TryFrom<String> for PhaseKind {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl PhaseKind {
    /// Every variant, in declaration order.
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    /// The variant's text: what `Display` prints and `FromStr` and serde accept.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
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
    /// The reviews of the review pass that asked for changes.
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
        /// The reviews of this review pass, in the order asked.
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

    /// Checks the reviews this phase carries: a review pass names no reviewer and no task twice, and an approved item
    /// carries at least one review, each an approval.
    ///
    /// # Errors
    /// [`InvalidReviews`] for the first review that does not fit.
    pub fn check_reviews(&self) -> Result<(), InvalidReviews> {
        match self {
            Self::InReview { reviews, .. } => check_review_pass(reviews),
            Self::Approved { reviews, .. } => {
                check_review_pass(reviews)?;
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

/// Checks that `pass` names no reviewer and no task twice.
fn check_review_pass(pass: &[Review]) -> Result<(), InvalidReviews> {
    let mut reviewers = HashSet::new();
    let mut tasks = HashSet::new();
    for review in pass {
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

/// Checks that `pass` is the reviews of an approved item: some, and every one an approval.
fn check_approving(pass: &[Review]) -> Result<(), InvalidReviews> {
    if pass.is_empty() {
        return Err(InvalidReviews::ApprovedWithoutReviews);
    }
    match pass.iter().find(|review| !approves(review)) {
        Some(review) => Err(InvalidReviews::ApprovedNotByAll {
            reviewer: review.reviewer.clone(),
        }),
        None => Ok(()),
    }
}

/// Checks that `pass` can be the reviews of a fix pass: a valid, non-empty review pass that is not wholly approving and
/// contains a submitted request for changes.
fn check_fix_pass(pass: &[Review]) -> Result<(), InvalidReviews> {
    check_review_pass(pass)?;
    if pass.is_empty() {
        return Err(InvalidReviews::FixPassWithoutReviews);
    }
    if pass.iter().all(approves) {
        return Err(InvalidReviews::FixPassApproved);
    }
    if !pass.iter().any(|review| {
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

/// A rejection of an item: its lead sent the member's work back instead of taking it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    /// When the lead's run ended with the line.
    at: Timestamp,
    /// What the lead said, from the bus.
    reason: String,
    /// The run of the item that counts the rejection as carried out on the branch; absent until it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recorded_in: Option<NonZeroU32>,
    /// The part of a split item the work belonged to; absent for an item that is not split and in states written before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<super::split::PartKey>,
    /// The run of the member that ended `DONE` and was sent back; absent in states written before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    of_run: Option<crate::ids::RunId>,
}

impl Rejection {
    /// When the lead's run ended with the line.
    #[must_use]
    pub const fn at(&self) -> Timestamp {
        self.at
    }

    /// What the lead said, from the bus.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The part of a split item the work belonged to; `None` for an item that is not split.
    #[must_use]
    pub const fn part(&self) -> Option<super::split::PartKey> {
        self.part
    }

    /// The run of the member that ended `DONE` and was sent back, when the state kept it.
    #[must_use]
    pub const fn of_run(&self) -> Option<&crate::ids::RunId> {
        self.of_run.as_ref()
    }

    /// The run of the item that counts it; `None` while the member has not started one after it.
    #[must_use]
    pub const fn counted_in(&self) -> Option<NonZeroU32> {
        self.recorded_in
    }
}

/// Why [`ItemProgress::record_rejection`] or [`ItemProgress::next_run_started`] refused to count a rejection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RejectionError {
    /// The item is not in progress or being fixed: nobody's work can be sent back.
    #[error("item {item} is not worked on")]
    NotWorkedOn {
        /// The item.
        item: ItemId,
    },
    /// The lead said nothing that can be shown as the reason.
    #[error("item {item}: the rejection has no reason")]
    NoReason {
        /// The item.
        item: ItemId,
    },
    /// A rejection at this time is recorded already.
    #[error("item {item}: the rejection of {at} is counted already")]
    AlreadyCounted {
        /// The item.
        item: ItemId,
        /// When it was recorded.
        at: Timestamp,
    },
    /// The run that would count the rejection started before the rejection was recorded.
    #[error("item {item}: the run that started at {run_started_at} is before the rejection of {counted}")]
    RunBeforeRejection {
        /// The item.
        item: ItemId,
        /// When the run started.
        run_started_at: Timestamp,
        /// When the rejection to count was recorded.
        counted: Timestamp,
    },
}

/// An item's progress: its phase, the fix passes behind it and its last gate result.
///
/// The phase changes only through [`ItemProgress::advance`], which follows [`PhaseKind::successors`].
///
/// Reading checks what the item's own constructors check: a phase with its data ([`ItemPhase`]), a fixing item with
/// a fix pass, and that every fix pass was asked for by reviews that fit it.
/// The mutually exclusive conflict pass accounting state for an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConflictPassHead {
    /// A conflict head for which a provisional pass is currently reserved.
    Provisional(CommitId),
    /// A conflict head for which the pass was refunded.
    Refunded(CommitId),
    /// A conflict head for which the pass was recorded as published.
    Accounted(CommitId),
}

/// An item's progress inside a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProgress {
    /// The item's number in the task list.
    id: ItemId,
    /// Where the item is.
    phase: ItemPhase,
    /// The fix passes so far, oldest first; their number is what the configured limit is compared with.
    fix_passes: Vec<FixPass>,
    /// The last gate result for this item, if the gate has run.
    pub gate: Option<GateResult>,
    /// The numbered protocol, absent for rounds opened by an earlier daemon.
    findings: Option<Findings>,
    /// The lead passes spent on merging the base into the branch after GitHub refused the merge for a conflict; the
    /// configured `[merge] conflict_passes` is what this is compared with. Absent while none is spent.
    merge_passes: u32,
    /// How many times the review of this item was reopened after a merge conflict resolution changed files beyond the
    /// task's allowances; absent while zero.
    conflict_verifications: u32,
    /// The approved head GitHub refused to merge before the current conflict pass or question, when one is in progress.
    conflict_approved_head: Option<CommitId>,
    /// The conflict pass head state (provisional, refunded, or accounted); absent when none.
    conflict_head: Option<ConflictPassHead>,
    /// The merge conflict pass in progress, if any; absent while the merge of the approved pull request is not in
    /// trouble.
    merge_conflict: Option<MergeConflict>,
    /// How many times publication of this item was tried; absent while zero.
    publish_tries: u32,
    /// Complete ownership of the publication question, absent while none is open.
    publish_question: Option<PublishQuestion>,
    /// The rejections the lead made since the agent that held the item last had none, oldest first; absent while none.
    rejections: Option<Vec<Rejection>>,
}

#[derive(Serialize)]
struct ItemProgressWireRef<'a> {
    id: ItemId,
    phase: &'a ItemPhase,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fix_passes: &'a Vec<FixPass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<&'a GateResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    findings: Option<&'a Findings>,
    #[serde(skip_serializing_if = "is_zero")]
    merge_passes: u32,
    #[serde(skip_serializing_if = "is_zero")]
    conflict_verifications: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict_approved_head: Option<&'a CommitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict_refunded_head: Option<&'a CommitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict_accounted_head: Option<&'a CommitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict_provisional_head: Option<&'a CommitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_conflict: Option<&'a MergeConflict>,
    #[serde(skip_serializing_if = "is_zero")]
    publish_tries: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    publish_question: Option<&'a PublishQuestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rejections: Option<&'a Vec<Rejection>>,
}

impl Serialize for ItemProgress {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (provisional, refunded, accounted) = match &self.conflict_head {
            Some(ConflictPassHead::Provisional(head)) => (Some(head), None, None),
            Some(ConflictPassHead::Refunded(head)) => (None, Some(head), None),
            Some(ConflictPassHead::Accounted(head)) => (None, None, Some(head)),
            None => (None, None, None),
        };
        ItemProgressWireRef {
            id: self.id,
            phase: &self.phase,
            fix_passes: &self.fix_passes,
            gate: self.gate.as_ref(),
            findings: self.findings.as_ref(),
            merge_passes: self.merge_passes,
            conflict_verifications: self.conflict_verifications,
            conflict_approved_head: self.conflict_approved_head.as_ref(),
            conflict_refunded_head: refunded,
            conflict_accounted_head: accounted,
            conflict_provisional_head: provisional,
            merge_conflict: self.merge_conflict.as_ref(),
            publish_tries: self.publish_tries,
            publish_question: self.publish_question.as_ref(),
            rejections: self.rejections.as_ref(),
        }
        .serialize(serializer)
    }
}

/// Exact ownership of a publication question; legacy questions may lack a recorded branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishQuestion {
    /// Task whose questions file owns the answer.
    task: TaskId,
    /// Number within that task.
    number: NonZeroU32,
    /// When the question was registered.
    at: Timestamp,
    /// Publication destination, absent only for older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<BranchName>,
}

/// Both supported on-disk publication question formats.
#[derive(Deserialize)]
#[serde(untagged)]
enum PublishQuestionWire {
    /// A complete current identity.
    Current(PublishQuestion),
    /// Earlier versions stored the timestamp separately from the identity.
    Legacy(Timestamp),
}

/// A merge of an approved pull request that GitHub refused for a conflict, while the harness works on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MergeConflict {
    /// The lead was sent to merge the base into the branch of the approved pull request; `approved_head` is the commit
    /// GitHub refused to merge, and `at` is when the lead was sent.
    MergingBase {
        /// The head the approval is for.
        approved_head: CommitId,
        /// When the lead was sent.
        at: Timestamp,
    },
    /// The passes are spent or the lead gave no result: the lead asks the human, once, in a questions file.
    AskingHuman {
        /// When the question was put to the lead.
        at: Timestamp,
    },
}

/// Whether a count is zero; a count of zero is not written.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// Backward-compatible reading of item progress.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// See [`ItemProgress`]; old rounds keep the verdict-only protocol.
    #[serde(default)]
    findings: Option<Findings>,
    /// See [`ItemProgress`]; a state written before merge passes has none.
    #[serde(default)]
    merge_passes: u32,
    /// See [`ItemProgress`]; a state written before conflict verifications has none.
    #[serde(default)]
    conflict_verifications: u32,
    /// See [`ItemProgress`]; a state written without a recorded conflict head has none.
    #[serde(default)]
    conflict_approved_head: Option<CommitId>,
    /// See [`ItemProgress`]; a state written without a refunded conflict head has none.
    #[serde(default)]
    conflict_refunded_head: Option<CommitId>,
    /// See [`ItemProgress`]; a state written without an accounted conflict head has none.
    #[serde(default)]
    conflict_accounted_head: Option<CommitId>,
    /// See [`ItemProgress`]; a state written without a provisional conflict head has none.
    #[serde(default)]
    conflict_provisional_head: Option<CommitId>,
    /// See [`ItemProgress`]; a state written before merge conflicts has none.
    #[serde(default)]
    merge_conflict: Option<MergeConflict>,
    /// See [`ItemProgress`]; a state written before publish tries has none.
    #[serde(default)]
    publish_tries: u32,
    /// See [`ItemProgress`]; a state written before publication questions has none.
    #[serde(default)]
    publish_question: Option<PublishQuestionWire>,
    /// See [`ItemProgress`]; a state written before publication question task has none.
    #[serde(default)]
    publish_question_task: Option<TaskId>,
    /// See [`ItemProgress`]; a state written before publication question number has none.
    #[serde(default)]
    publish_question_number: Option<NonZeroU32>,
    /// See [`ItemProgress`]; a state written before a lead could return an item has none.
    #[serde(default)]
    rejections: Option<Vec<Rejection>>,
}

impl<'de> Deserialize<'de> for ItemProgress {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ItemProgressWire::deserialize(deserializer)?;
        let publish_question = match wire.publish_question {
            Some(PublishQuestionWire::Current(question)) => Some(question),
            Some(PublishQuestionWire::Legacy(at)) => match (wire.publish_question_task, wire.publish_question_number) {
                (Some(task), Some(number)) => Some(PublishQuestion {
                    task,
                    number,
                    at,
                    branch: None,
                }),
                // Incomplete old records cannot authorize any task's answer. Leave them available for escalation.
                _ => None,
            },
            None => None,
        };
        let conflict_head = match (
            wire.conflict_provisional_head,
            wire.conflict_refunded_head,
            wire.conflict_accounted_head,
        ) {
            (None, None, None) => None,
            (Some(head), None, None) => Some(ConflictPassHead::Provisional(head)),
            (None, Some(head), None) => Some(ConflictPassHead::Refunded(head)),
            (None, None, Some(head)) => Some(ConflictPassHead::Accounted(head)),
            _ => {
                return Err(serde::de::Error::custom(
                    "item progress cannot have multiple contradictory conflict pass heads simultaneously",
                ));
            }
        };
        let value = Self {
            id: wire.id,
            phase: wire.phase,
            fix_passes: wire.fix_passes,
            gate: wire.gate,
            findings: wire.findings,
            merge_passes: wire.merge_passes,
            conflict_verifications: wire.conflict_verifications,
            conflict_approved_head: wire.conflict_approved_head,
            conflict_head,
            merge_conflict: wire.merge_conflict,
            publish_tries: wire.publish_tries,
            publish_question,
            rejections: wire.rejections,
        };
        check_item_progress(&value).map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

/// An item's progress inside a team, to change through its operations only.
///
/// It reads like an [`ItemProgress`] (it dereferences to one) but is not a `&mut ItemProgress`: the progress cannot be
/// overwritten with another, so the phase changes only through [`ItemMut::advance`], which keeps to the table of
/// phase changes and checks the reviews.
pub type ItemMut<'a> = CheckedMut<'a, ItemProgress>;

impl CheckedMut<'_, ItemProgress> {
    /// See [`ItemProgress::begin_merge_conflict`].
    pub fn begin_merge_conflict(&mut self, approved_head: CommitId, at: Timestamp) -> u32 {
        self.value.begin_merge_conflict(approved_head, at)
    }

    /// See [`ItemProgress::end_merge_conflict`].
    pub fn end_merge_conflict(&mut self) {
        self.value.end_merge_conflict();
    }

    /// See [`ItemProgress::ask_human_about_merge_conflict`].
    pub fn ask_human_about_merge_conflict(&mut self, at: Timestamp) {
        self.value.ask_human_about_merge_conflict(at);
    }

    /// See [`ItemProgress::record_publish_try`].
    pub fn record_publish_try(&mut self) -> u32 {
        self.value.record_publish_try()
    }

    /// See [`ItemProgress::reset_publish_tries`].
    pub fn reset_publish_tries(&mut self) {
        self.value.reset_publish_tries();
    }

    /// See [`ItemProgress::record_publish_question_branch`].
    pub fn record_publish_question_branch(
        &mut self,
        task: TaskId,
        number: NonZeroU32,
        at: Timestamp,
        branch: BranchName,
    ) {
        self.value.record_publish_question_branch(task, number, at, branch);
    }

    /// See [`ItemProgress::clear_publish_question`].
    pub fn clear_publish_question(&mut self) {
        self.value.clear_publish_question();
    }

    /// See [`ItemProgress::record_published_conflict_pass`].
    pub fn record_published_conflict_pass(&mut self) -> u32 {
        self.value.record_published_conflict_pass()
    }

    /// See [`ItemProgress::refund_conflict_pass`].
    pub fn refund_conflict_pass(&mut self) -> u32 {
        self.value.refund_conflict_pass()
    }

    /// See [`ItemProgress::has_provisional_conflict_pass`].
    #[must_use]
    pub fn has_provisional_conflict_pass(&self) -> bool {
        self.value.has_provisional_conflict_pass()
    }

    /// See [`ItemProgress::conflict_provisional_head`].
    #[must_use]
    pub fn conflict_provisional_head(&self) -> Option<&CommitId> {
        self.value.conflict_provisional_head()
    }

    /// See [`ItemProgress::set_conflict_approved_head`].
    pub fn set_conflict_approved_head(&mut self, approved_head: CommitId) {
        self.value.set_conflict_approved_head(approved_head);
    }

    /// See [`ItemProgress::reopen_review_after_conflict`].
    ///
    /// # Errors
    /// As [`ItemProgress::reopen_review_after_conflict`].
    pub fn reopen_review_after_conflict(&mut self) -> Result<(), AdvanceError> {
        self.value.reopen_review_after_conflict()
    }

    /// See [`ItemProgress::advance`]. An item in review is approved only with the pull request and the reviews it has:
    /// the outcomes reach it through [`CheckedMut::submit_review`], not through the phase handed in. The pull request
    /// that was reviewed is also the one the item is fixed on and merged with.
    ///
    /// # Errors
    /// [`AdvanceError`], as for [`ItemProgress::advance`]; [`AdvanceError::ReviewPassNotKept`] when an approval carries
    /// another pull request, other reviews, or an outcome that was not submitted, or when a fix pass or a merge
    /// carries another pull request than the reviewed one.
    pub fn advance(&mut self, next: ItemPhase) -> Result<(), AdvanceError> {
        self.value.check_review_pass_kept(&next)?;
        self.value.advance(next)
    }

    /// See [`ItemProgress::reassign`].
    ///
    /// # Errors
    /// [`NotWorkedOn`] when the item is in any other phase than in progress or fixing.
    pub fn reassign(&mut self, assignee: AgentName, branch: BranchName) -> Result<(), NotWorkedOn> {
        self.value.reassign(assignee, branch)
    }

    /// See [`ItemProgress::record_rejection`].
    ///
    /// # Errors
    /// [`RejectionError`], as for [`ItemProgress::record_rejection`].
    pub fn record_rejection(&mut self, reason: String, at: Timestamp) -> Result<Option<Timestamp>, RejectionError> {
        self.value.record_rejection(reason, at)
    }

    /// See [`ItemProgress::record_rejection_of`].
    ///
    /// # Errors
    /// [`RejectionError`], as for [`ItemProgress::record_rejection`].
    pub fn record_rejection_of(
        &mut self,
        reason: String,
        at: Timestamp,
        part: Option<super::split::PartKey>,
        run: Option<crate::ids::RunId>,
    ) -> Result<Option<Timestamp>, RejectionError> {
        self.value.record_rejection_of(reason, at, part, run)
    }

    /// See [`ItemProgress::next_run_started`].
    ///
    /// # Errors
    /// [`RejectionError`], as for [`ItemProgress::next_run_started`].
    pub fn next_run_started(&mut self, run: NonZeroU32, at: Timestamp) -> Result<u32, RejectionError> {
        self.value.next_run_started(run, at)
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

    /// See [`ItemProgress::ask_reviews`].
    ///
    /// # Errors
    /// [`AskReviewsError`], as for [`ItemProgress::ask_reviews`].
    pub fn ask_reviews(&mut self, reviews: Vec<Review>) -> Result<(), AskReviewsError> {
        self.value.ask_reviews(reviews)
    }

    /// See [`ItemProgress::replace_review`].
    ///
    /// # Errors
    /// [`ReplaceReviewError`], as for [`ItemProgress::replace_review`].
    pub fn replace_review(&mut self, gone: &TaskId, with: Review) -> Result<(), ReplaceReviewError> {
        self.value.replace_review(gone, with)
    }

    /// Completes an unchanged pass whose blockers were all independently not confirmed.
    ///
    /// # Errors
    /// [`AdvanceError::Transition`] unless this is a fixing item with no remaining blocker,
    /// every opinion is settled, and `head` equals the last reviewed commit.
    pub fn complete_unconfirmed(&mut self, head: &CommitId, at: Timestamp) -> Result<(), AdvanceError> {
        use crate::decide::opinion::{Next, Tip, after_opinions};
        let from = self.value.phase.kind();
        let pass = self
            .value
            .fix_passes
            .iter()
            .try_fold(0_u32, |count, _| count.checked_add(1))
            .and_then(std::num::NonZeroU32::new)
            .ok_or(InvalidAdvance {
                item: self.value.id,
                from,
                to: PhaseKind::Done,
            })?;
        if let ItemPhase::Fixing { pr, .. } = &self.value.phase
            && let Some(ledger) = self.value.findings.as_ref()
            && ledger.verification().and_then(|context| context.reviewed_head.as_ref()) == Some(head)
            && after_opinions(ledger, pass, &ledger.fixed_answers(pass), Tip::Unchanged) == Next::DecideUnchanged
        {
            self.value.phase = ItemPhase::Done {
                completion: Completion::Merged { pr: pr.clone(), at },
            };
            return Ok(());
        }
        Err(InvalidAdvance {
            item: self.value.id,
            from,
            to: PhaseKind::Done,
        }
        .into())
    }

    /// Enables numbered findings for a newly opened pull request.
    pub fn start_findings(&mut self) {
        self.value.findings = Some(Findings::default());
    }

    /// Changes the ledger of a pull request using the numbered protocol.
    pub fn findings_mut(&mut self) -> Option<&mut Findings> {
        self.value.findings.as_mut()
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
    /// One reviewer has two reviews in one review pass.
    #[error("the reviewer {reviewer} has two reviews in one review pass")]
    DuplicateReviewer {
        /// The repeated reviewer.
        reviewer: InvestigatorName,
    },
    /// One reviewer task has two reviews in one review pass.
    #[error("the task {task} has two reviews in one review pass")]
    DuplicateTask {
        /// The repeated task.
        task: TaskId,
    },
    /// A fix pass records no review: nothing sent the item back.
    #[error("a fix pass must record the reviews that sent the item back, and has none")]
    FixPassWithoutReviews,
    /// Every review of the review pass approved: the item is approved, not sent back.
    #[error("every review of the review pass approved: the item cannot go back for a fix pass")]
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
    /// The reviews of the review pass do not fit the phase.
    #[error("item {item}: {source}")]
    Reviews {
        /// The item.
        item: ItemId,
        /// What is wrong with the reviews.
        #[source]
        source: InvalidReviews,
    },
    /// The change does not carry the review pass the item is in: an approval is not made of the pull request and the reviews
    /// as recorded, or a fix pass or a merge is not on the reviewed pull request.
    #[error(
        "item {item}: an approval carries the item's own pull request and reviews, and a fix pass or a merge its reviewed pull request, as recorded"
    )]
    ReviewPassNotKept {
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

/// Why a review could not be given to another reviewer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplaceReviewError {
    /// Only an item in review has reviews that are still to come.
    #[error("item {item} is {phase}: only an item in review has a review to give to another reviewer")]
    NotInReview {
        /// The item.
        item: ItemId,
        /// Its phase.
        phase: PhaseKind,
    },
    /// The review pass has no review of that task.
    #[error("item {item}: no review of this review pass was asked of {task}")]
    UnknownReview {
        /// The item.
        item: ItemId,
        /// The task.
        task: TaskId,
    },
    /// The review came: a review that has its outcome stays its reviewer's.
    #[error("item {item}: the review asked of {task} has its outcome, so it is not given to another reviewer")]
    AlreadySubmitted {
        /// The item.
        item: ItemId,
        /// The task.
        task: TaskId,
    },
    /// The review that takes the place has its outcome already.
    #[error("item {item}: the review by {reviewer} has its outcome already, so it cannot be asked for")]
    NotPending {
        /// The item.
        item: ItemId,
        /// The new reviewer.
        reviewer: InvestigatorName,
    },
    /// With the new review the pass would name a reviewer or a task twice.
    #[error("item {item}: the review pass would not be valid")]
    Reviews {
        /// The item.
        item: ItemId,
        /// What is wrong with it.
        #[source]
        source: InvalidReviews,
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
    /// The review pass asked no review of this task.
    #[error("item {item}: no review of the review pass was asked of task {task}")]
    UnknownReview {
        /// The item.
        item: ItemId,
        /// The task that is not a reviewer of the review pass.
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

/// Why the reviews of a pull request could not be asked for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AskReviewsError {
    /// Only an item in review has reviews to fill.
    #[error("item {item} is {phase}: only an item in review is asked for reviews")]
    NotInReview {
        /// The item.
        item: ItemId,
        /// Its phase.
        phase: PhaseKind,
    },
    /// The review pass has its reviews already.
    #[error("item {item}: the reviews of this review pass are asked for already")]
    AlreadyAsked {
        /// The item.
        item: ItemId,
    },
    /// No review was given: a request asks for at least one.
    #[error("item {item}: a request for reviews names no reviewer")]
    NoReviews {
        /// The item.
        item: ItemId,
    },
    /// A review given has its outcome already: only reviews that wait for theirs are asked for.
    #[error("item {item}: the review by {reviewer} has its outcome already, so it cannot be asked for")]
    NotPending {
        /// The item.
        item: ItemId,
        /// The reviewer whose review is not pending.
        reviewer: InvestigatorName,
    },
    /// The reviews do not fit one review pass.
    #[error("item {item}: {source}")]
    Reviews {
        /// The item.
        item: ItemId,
        /// What is wrong with them.
        #[source]
        source: InvalidReviews,
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
    /// The rejections of the lead since the agent that held the item last had none, oldest first; `None` for an item
    /// whose state was written before a lead could return work.
    #[must_use]
    pub fn rejections(&self) -> Option<&[Rejection]> {
        self.rejections.as_deref()
    }

    /// How many rejections of the item were counted against the limit `limit` of `[replacement] rejections`.
    #[must_use]
    pub fn counted_rejections(&self, limit: u32) -> crate::decide::rejection::Rejections {
        let counted = self
            .rejections
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|rejection| rejection.counted_in().is_some())
            .count();
        let counted = u32::try_from(counted).unwrap_or(u32::MAX);
        crate::decide::rejection::Rejections::new(counted, limit)
    }

    /// Records that the lead returned the item at `at` with `reason`, after the rejections recorded already: returns
    /// whether that makes a run of the item count the rejections recorded before `at` as carried out on the branch.
    ///
    /// # Errors
    /// [`RejectionError::NotWorkedOn`] when the item is not in progress or being fixed;
    /// [`RejectionError::NoReason`] when `reason` says nothing;
    /// [`RejectionError::AlreadyCounted`] when a rejection at `at` is recorded already.
    pub fn record_rejection(&mut self, reason: String, at: Timestamp) -> Result<Option<Timestamp>, RejectionError> {
        self.record_rejection_of(reason, at, None, None)
    }

    /// Records the rejection as [`ItemProgress::record_rejection`] does, with the `part` of the split item the work
    /// belonged to and the `run` of the member that ended `DONE` and was sent back.
    ///
    /// # Errors
    /// [`RejectionError`], as for [`ItemProgress::record_rejection`].
    pub fn record_rejection_of(
        &mut self,
        reason: String,
        at: Timestamp,
        part: Option<super::split::PartKey>,
        run: Option<crate::ids::RunId>,
    ) -> Result<Option<Timestamp>, RejectionError> {
        if !matches!(self.phase, ItemPhase::InProgress { .. } | ItemPhase::Fixing { .. }) {
            return Err(RejectionError::NotWorkedOn { item: self.id });
        }
        if reason.trim().is_empty() {
            return Err(RejectionError::NoReason { item: self.id });
        }
        let recorded = self.rejections.get_or_insert_with(Vec::new);
        if recorded.iter().any(|rejection| rejection.at == at) {
            return Err(RejectionError::AlreadyCounted { item: self.id, at });
        }
        let settled_at = recorded.last().map(|rejection| rejection.at);
        recorded.push(Rejection {
            at,
            reason,
            recorded_in: None,
            part,
            of_run: run,
        });
        Ok(settled_at)
    }

    /// How many rejections of the work of `part` were counted against the limit `limit`; `None` is the item that is not
    /// split.
    #[must_use]
    pub fn counted_rejections_of(
        &self,
        part: Option<super::split::PartKey>,
        limit: u32,
    ) -> crate::decide::rejection::Rejections {
        let counted = self
            .rejections
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|rejection| rejection.part == part && rejection.counted_in().is_some())
            .count();
        crate::decide::rejection::Rejections::new(u32::try_from(counted).unwrap_or(u32::MAX), limit)
    }

    /// Whether the work the member ended `DONE` in `run` was sent back already, whether the member ran again since or not.
    #[must_use]
    pub fn was_rejected(&self, run: &crate::ids::RunId) -> bool {
        self.rejections
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|rejection| rejection.of_run.as_ref() == Some(run))
    }

    /// Records that run `run` of the item started at `at`, the run the rejections recorded so far count on: returns
    /// how many of them that run counts.
    ///
    /// # Errors
    /// [`RejectionError::RunBeforeRejection`] when a rejection was recorded after the run started.
    pub fn next_run_started(&mut self, run: NonZeroU32, at: Timestamp) -> Result<u32, RejectionError> {
        let Some(recorded) = self.rejections.as_deref_mut() else {
            return Ok(0);
        };
        if recorded
            .iter()
            .any(|rejection| rejection.recorded_in.is_none() && rejection.at > at)
        {
            let rejection = recorded
                .iter()
                .find(|rejection| rejection.recorded_in.is_none() && rejection.at > at)
                .map(|rejection| rejection.at);
            return Err(RejectionError::RunBeforeRejection {
                item: self.id,
                run_started_at: at,
                counted: rejection.unwrap_or(at),
            });
        }
        // The run that starts is the run every rejection so far was settled by; the rejections present when it starts
        // are the ones it carries.
        let mut counted = 0_u32;
        for rejection in recorded.iter_mut() {
            if rejection.recorded_in.is_some() {
                continue;
            }
            rejection.recorded_in = Some(run);
            counted = counted.saturating_add(1);
        }
        Ok(counted)
    }

    /// Completes only a working item after the team checked its delivery claim.
    pub(super) fn complete_delivery(
        &mut self,
        round: &crate::ids::RoundId,
        pr: PullRequest,
        at: Timestamp,
    ) -> Result<(), super::CompleteDeliveryError> {
        if !matches!(self.phase, ItemPhase::InProgress { .. }) {
            return Err(super::CompleteDeliveryError::NotWorking { round: round.clone() });
        }
        self.phase = ItemPhase::Done {
            completion: Completion::Merged { pr, at },
        };
        Ok(())
    }

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
            findings: None,
            merge_passes: 0,
            conflict_verifications: 0,
            conflict_approved_head: None,
            conflict_head: None,
            merge_conflict: None,
            publish_tries: 0,
            publish_question: None,
            rejections: None,
        }
    }

    /// How many times publication of this item was tried.
    #[must_use]
    pub const fn publish_tries(&self) -> u32 {
        self.publish_tries
    }

    /// Increments the publication attempt counter and returns the new count.
    pub fn record_publish_try(&mut self) -> u32 {
        self.publish_tries = self.publish_tries.saturating_add(1);
        self.publish_tries
    }

    /// Resets the publication attempt counter.
    pub fn reset_publish_tries(&mut self) {
        self.publish_tries = 0;
    }

    /// When the publication failure question was registered.
    #[must_use]
    pub fn publish_question(&self) -> Option<Timestamp> {
        self.publish_question.as_ref().map(|q| q.at)
    }

    /// Task that owns the publication answer.
    #[must_use]
    pub fn publish_question_task(&self) -> Option<&TaskId> {
        self.publish_question.as_ref().map(|q| &q.task)
    }

    /// Number of the publication question.
    #[must_use]
    pub fn publish_question_number(&self) -> Option<NonZeroU32> {
        self.publish_question.as_ref().map(|q| q.number)
    }

    /// Destination recorded with the publication question, if known.
    #[must_use]
    pub fn publish_question_branch(&self) -> Option<&BranchName> {
        self.publish_question.as_ref().and_then(|q| q.branch.as_ref())
    }

    /// Records the exact publication destination with its question identity.
    pub fn record_publish_question_branch(
        &mut self,
        task: TaskId,
        number: NonZeroU32,
        at: Timestamp,
        branch: BranchName,
    ) {
        self.publish_question = Some(PublishQuestion {
            task,
            number,
            at,
            branch: Some(branch),
        });
    }

    /// Clears the publication question identity together.
    pub fn clear_publish_question(&mut self) {
        self.publish_question = None;
    }

    /// Records that a merge conflict pass was published and increments spent merge passes.
    pub fn record_published_conflict_pass(&mut self) -> u32 {
        if let Some(ref head) = self.conflict_approved_head.clone() {
            if self.conflict_accounted_head() == Some(head) {
                return self.merge_passes;
            }
            if self.conflict_refunded_head() == Some(head) {
                self.merge_passes = self.merge_passes.saturating_add(1);
            }
            self.conflict_head = Some(ConflictPassHead::Accounted(head.clone()));
        }
        self.merge_passes
    }

    /// Refunds a demonstrably reserved conflict pass on publication failure and returns the new count.
    /// If no provisional pass was reserved (e.g. an old state or already refunded), the count is preserved untouched.
    pub fn refund_conflict_pass(&mut self) -> u32 {
        if let Some(ref head) = self.conflict_approved_head.clone() {
            if self.conflict_accounted_head() == Some(head) {
                return self.merge_passes;
            }
            if self.conflict_provisional_head() == Some(head) {
                self.merge_passes = self.merge_passes.saturating_sub(1);
            }
            self.conflict_head = Some(ConflictPassHead::Refunded(head.clone()));
        }
        self.merge_passes
    }

    getter!("The item's number in the task list.", pub const fn id(&self) -> ItemId = self.id);

    getter!("Where the item is.", pub const fn phase(&self) -> &ItemPhase = &self.phase);

    getter!("The item status the phase shows.", pub const fn status(&self) -> ItemStatus = self.phase.kind().status());

    getter!("The fix passes so far, oldest first.", pub fn fix_passes(&self) -> &[FixPass] = &self.fix_passes);

    getter!("The findings ledger, or none for a legacy round.", pub fn findings(&self) -> Option<&Findings> = self.findings.as_ref());

    /// The lead passes spent on merge conflicts so far; zero for an item that never met one.
    #[must_use]
    pub const fn merge_passes(&self) -> u32 {
        self.merge_passes
    }

    /// How many review passes the item has completed before its current one: its fix passes and the merge conflict
    /// passes or reopened conflict verifications it has spent.
    #[must_use]
    pub fn completed_review_passes(&self) -> usize {
        let conflict_passes = if self.conflict_verifications > 0 {
            self.conflict_verifications
        } else {
            self.merge_passes
        };
        self.fix_passes
            .len()
            .saturating_add(usize::try_from(conflict_passes).unwrap_or(usize::MAX))
    }

    /// The approved head GitHub refused to merge before the current conflict pass or question, if recorded.
    #[must_use]
    pub const fn conflict_approved_head(&self) -> Option<&CommitId> {
        self.conflict_approved_head.as_ref()
    }

    /// The conflict head for which the pass was recorded as published, if any.
    #[must_use]
    pub fn conflict_accounted_head(&self) -> Option<&CommitId> {
        match &self.conflict_head {
            Some(ConflictPassHead::Accounted(head)) => Some(head),
            _ => None,
        }
    }

    /// The conflict head for which the pass was refunded, if any.
    #[must_use]
    pub fn conflict_refunded_head(&self) -> Option<&CommitId> {
        match &self.conflict_head {
            Some(ConflictPassHead::Refunded(head)) => Some(head),
            _ => None,
        }
    }

    /// The conflict head for which a provisional pass is currently reserved, if any.
    #[must_use]
    pub fn conflict_provisional_head(&self) -> Option<&CommitId> {
        match &self.conflict_head {
            Some(ConflictPassHead::Provisional(head)) => Some(head),
            _ => None,
        }
    }

    /// Whether a provisional conflict pass is currently reserved and unconfirmed.
    #[must_use]
    pub fn has_provisional_conflict_pass(&self) -> bool {
        matches!(&self.conflict_head, Some(ConflictPassHead::Provisional(_)))
    }

    /// Records the approved head GitHub refused to merge before the current conflict pass or question.
    pub fn set_conflict_approved_head(&mut self, approved_head: CommitId) {
        self.conflict_approved_head = Some(approved_head);
    }

    /// The merge conflict pass in progress, if any.
    #[must_use]
    pub const fn merge_conflict(&self) -> Option<&MergeConflict> {
        self.merge_conflict.as_ref()
    }

    /// Sends the lead to merge the base into the branch of `approved_head`: one more pass is spent and the pass is
    /// recorded at `at`. Returns the passes spent so far, this one included.
    pub fn begin_merge_conflict(&mut self, approved_head: CommitId, at: Timestamp) -> u32 {
        self.conflict_approved_head = Some(approved_head.clone());
        self.conflict_head = Some(ConflictPassHead::Provisional(approved_head.clone()));
        self.merge_conflict = Some(MergeConflict::MergingBase { approved_head, at });
        self.spend_merge_pass()
    }

    /// Ends the merge conflict pass in progress; the passes spent stay counted.
    pub fn end_merge_conflict(&mut self) {
        self.merge_conflict = None;
        self.conflict_approved_head = None;
        self.conflict_head = None;
    }

    /// Reopens the review of an approved item whose branch the lead changed beyond the merge of the base: the reviewers
    /// look at the pull request again and no review is kept.
    ///
    /// # Errors
    /// [`AdvanceError::Transition`] unless the item is approved.
    pub fn reopen_review_after_conflict(&mut self) -> Result<(), AdvanceError> {
        let ItemPhase::Approved { pr, .. } = &self.phase else {
            return Err(InvalidAdvance {
                item: self.id,
                from: self.phase.kind(),
                to: PhaseKind::InReview,
            }
            .into());
        };
        self.conflict_verifications = self.conflict_verifications.saturating_add(1);
        self.phase = ItemPhase::InReview {
            pr: pr.clone(),
            reviews: Vec::new(),
        };
        Ok(())
    }

    /// Records that the human is asked about the merge conflict at `at`, once.
    pub fn ask_human_about_merge_conflict(&mut self, at: Timestamp) {
        self.merge_conflict = Some(MergeConflict::AskingHuman { at });
    }

    /// Counts one more lead pass on a merge conflict and returns the number spent so far, including this one. A count
    /// that would pass `u32::MAX` stays there: no configured limit is that large.
    pub fn spend_merge_pass(&mut self) -> u32 {
        self.merge_passes = self.merge_passes.saturating_add(1);
        self.merge_passes
    }

    /// Moves the item to `next`. Leaving `in_review` for `fixing` records the review pass's reviews as a fix pass.
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

    /// Checks that the review pass an item has is kept: an approval carries exactly its pull request and reviews, and the
    /// pull request that was reviewed is the one the item goes back to be fixed on and is merged with. Any other
    /// change is left to [`ItemProgress::advance`].
    ///
    /// # Errors
    /// [`AdvanceError::ReviewPassNotKept`] when `next` carries another review pass than the one recorded.
    fn check_review_pass_kept(&self, next: &ItemPhase) -> Result<(), AdvanceError> {
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
        Err(AdvanceError::ReviewPassNotKept { item: self.id })
    }

    /// Records the verdict that the reviewer task `task` reported for the item's pull request.
    ///
    /// # Errors
    /// [`SubmitReviewError`]: the item is not in review, no review of the review pass was asked of `task`, or that review
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

    /// Fills the review pass of an item in review with the reviews asked of its reviewers. The review pass starts empty when the
    /// pull request is opened and is filled once, when reviewers are free to take it.
    ///
    /// # Errors
    /// [`AskReviewsError::NotInReview`] when the item is in another phase; [`AskReviewsError::AlreadyAsked`] when its
    /// pass has reviews; [`AskReviewsError::NoReviews`] for an empty request; [`AskReviewsError::NotPending`] for a
    /// review that has its outcome; [`AskReviewsError::Reviews`] when a reviewer or a task is named twice.
    pub fn ask_reviews(&mut self, reviews: Vec<Review>) -> Result<(), AskReviewsError> {
        let item = self.id;
        let ItemPhase::InReview { reviews: pass, .. } = &mut self.phase else {
            return Err(AskReviewsError::NotInReview {
                item,
                phase: self.phase.kind(),
            });
        };
        if !pass.is_empty() {
            return Err(AskReviewsError::AlreadyAsked { item });
        }
        if reviews.is_empty() {
            return Err(AskReviewsError::NoReviews { item });
        }
        if let Some(answered) = reviews
            .iter()
            .find(|review| !matches!(review.outcome, ReviewOutcome::Pending))
        {
            return Err(AskReviewsError::NotPending {
                item,
                reviewer: answered.reviewer.clone(),
            });
        }
        check_review_pass(&reviews).map_err(|source| AskReviewsError::Reviews { item, source })?;
        *pass = reviews;
        Ok(())
    }

    /// Gives the review that was asked of the reviewer task `gone`, and has not come, to another reviewer: `with` takes
    /// its place in the review pass. The other reviews of the pass stay as they are.
    ///
    /// # Errors
    /// [`ReplaceReviewError`]: the item is not in review, no review of the pass was asked of `gone`, that review has its
    /// outcome, `with` has one, or `with` names a reviewer or a task the pass already has.
    pub fn replace_review(&mut self, gone: &TaskId, with: Review) -> Result<(), ReplaceReviewError> {
        let item = self.id;
        let phase = self.phase.kind();
        let ItemPhase::InReview { reviews, .. } = &mut self.phase else {
            return Err(ReplaceReviewError::NotInReview { item, phase });
        };
        let Some(index) = reviews.iter().position(|review| &review.task == gone) else {
            return Err(ReplaceReviewError::UnknownReview {
                item,
                task: gone.clone(),
            });
        };
        if reviews
            .get(index)
            .is_some_and(|review| !matches!(review.outcome, ReviewOutcome::Pending))
        {
            return Err(ReplaceReviewError::AlreadySubmitted {
                item,
                task: gone.clone(),
            });
        }
        if !matches!(with.outcome, ReviewOutcome::Pending) {
            return Err(ReplaceReviewError::NotPending {
                item,
                reviewer: with.reviewer,
            });
        }
        // The pass is checked as it would be, and changed only when it is valid.
        let mut next = reviews.clone();
        if let Some(place) = next.get_mut(index) {
            *place = with;
        }
        check_review_pass(&next).map_err(|source| ReplaceReviewError::Reviews { item, source })?;
        *reviews = next;
        Ok(())
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

    /// Restores a branchless question as written before destinations were persisted.
    fn record_legacy_publish_question(item: &mut ItemProgress, task: &TaskId, number: NonZeroU32, at: Timestamp) {
        let mut fixture = serde_json::to_value(&*item).unwrap();
        fixture["publish_question"] = json!({ "task": task, "number": number, "at": at });
        *item = serde_json::from_value(fixture).unwrap();
    }

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
                ..Self::new(wire.id, ItemMark::Open)
            };
            check_has_fix_pass(&progress)?;
            Ok(progress)
        }
    }

    /// The id of the item of these tests.
    fn id() -> ItemId {
        "1.2".parse().unwrap()
    }

    /// The two reviews of a review pass, both asked for.
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
    fn a_new_item_has_spent_no_merge_pass_and_writes_none() {
        let item = progress_in(PhaseKind::Approved);

        assert_eq!(item.merge_passes(), 0);
        let text = serde_json::to_string(&item).unwrap();
        assert!(!text.contains("merge_passes"), "{text}");
    }

    #[test]
    fn a_merge_conflict_pass_is_recorded_written_and_ended() {
        let mut item = progress_in(PhaseKind::Approved);
        let head: CommitId = "1111111111111111111111111111111111111111".parse().unwrap();
        let at = at(1);

        assert_eq!(item.begin_merge_conflict(head.clone(), at), 1);
        let text = serde_json::to_string(&item).unwrap();
        let back: ItemProgress = serde_json::from_str(&text).unwrap();
        assert_eq!(
            back.merge_conflict(),
            Some(&MergeConflict::MergingBase {
                approved_head: head,
                at,
            })
        );
        assert_eq!(back.merge_passes(), 1);

        item.end_merge_conflict();
        assert_eq!(item.merge_conflict(), None);
        assert_eq!(item.merge_passes(), 1);
    }

    #[test]
    fn reopening_the_review_after_a_conflict_keeps_no_review_and_needs_an_approval() {
        let mut item = progress_in(PhaseKind::Approved);

        item.reopen_review_after_conflict().unwrap();
        assert_eq!(item.phase().kind(), PhaseKind::InReview);
        assert!(matches!(item.phase(), ItemPhase::InReview { reviews, .. } if reviews.is_empty()));

        let mut pending = progress_in(PhaseKind::InProgress);
        assert_eq!(
            pending.reopen_review_after_conflict(),
            Err(AdvanceError::Transition(InvalidAdvance {
                item: id(),
                from: PhaseKind::InProgress,
                to: PhaseKind::InReview,
            }))
        );
    }

    #[test]
    fn asking_the_human_about_a_merge_conflict_is_recorded() {
        let mut item = progress_in(PhaseKind::Approved);
        let at = at(1);

        item.ask_human_about_merge_conflict(at);

        assert_eq!(item.merge_conflict(), Some(&MergeConflict::AskingHuman { at }));
    }

    #[test]
    fn a_spent_merge_pass_is_written_and_read_back() {
        let mut item = progress_in(PhaseKind::Approved);

        assert_eq!(item.spend_merge_pass(), 1);
        let text = serde_json::to_string(&item).unwrap();
        let back: ItemProgress = serde_json::from_str(&text).unwrap();

        assert_eq!(back.merge_passes(), 1);
    }

    #[test]
    fn publish_tries_and_question_are_persisted_and_read_back() {
        let mut item = progress_in(PhaseKind::InProgress);
        assert_eq!(item.publish_tries(), 0);
        assert_eq!(item.publish_question(), None);

        assert_eq!(item.record_publish_try(), 1);
        assert_eq!(item.record_publish_try(), 2);
        record_legacy_publish_question(
            &mut item,
            &"task-20261111-153001-k3x9".parse().unwrap(),
            NonZeroU32::new(3).unwrap(),
            at(100),
        );

        let text = serde_json::to_string(&item).unwrap();
        assert!(text.contains("\"publish_tries\":2"), "{text}");
        assert!(text.contains("\"publish_question\":"), "{text}");

        let back: ItemProgress = serde_json::from_str(&text).unwrap();
        assert_eq!(back.publish_tries(), 2);
        assert_eq!(back.publish_question(), Some(at(100)));

        item.reset_publish_tries();
        item.clear_publish_question();
        assert_eq!(item.publish_tries(), 0);
        assert_eq!(item.publish_question(), None);
    }

    #[test]
    fn publish_question_identity_stores_and_clears_task_and_number() {
        let mut item = progress_in(PhaseKind::InProgress);
        assert_eq!(item.publish_question_task(), None);
        assert_eq!(item.publish_question_number(), None);

        let t: TaskId = "task-20261111-153001-k3x9".parse().unwrap();
        let num = NonZeroU32::new(3).unwrap();
        record_legacy_publish_question(&mut item, &t, num, at(100));

        assert_eq!(item.publish_question(), Some(at(100)));
        assert_eq!(item.publish_question_task(), Some(&t));
        assert_eq!(item.publish_question_number(), Some(num));

        let text = serde_json::to_string(&item).unwrap();
        assert!(text.contains("\"task\":\"task-20261111-153001-k3x9\""), "{text}");
        assert!(text.contains("\"number\":3"), "{text}");

        let back: ItemProgress = serde_json::from_str(&text).unwrap();
        assert_eq!(back.publish_question(), Some(at(100)));
        assert_eq!(back.publish_question_task(), Some(&t));
        assert_eq!(back.publish_question_number(), Some(num));

        item.clear_publish_question();
        assert_eq!(item.publish_question(), None);
        assert_eq!(item.publish_question_task(), None);
        assert_eq!(item.publish_question_number(), None);
    }

    #[test]
    fn beginning_merge_conflict_spends_pass() {
        let mut item = progress_in(PhaseKind::Approved);
        assert_eq!(item.merge_passes(), 0);

        let head: CommitId = "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap();
        assert_eq!(item.begin_merge_conflict(head, at(10)), 1);
        assert_eq!(item.merge_passes(), 1);
    }

    #[test]
    fn publishing_conflict_resolution_records_conflict_pass_and_refund_restores_it() {
        let mut item = progress_in(PhaseKind::Approved);
        assert_eq!(item.merge_passes(), 0);

        let head: CommitId = "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap();
        item.begin_merge_conflict(head, at(10));
        assert_eq!(item.merge_passes(), 1);

        // A failed publication refunds the pass:
        assert_eq!(item.refund_conflict_pass(), 0);
        assert_eq!(item.merge_passes(), 0);

        // Repeated failure keeps passes untouched:
        assert_eq!(item.refund_conflict_pass(), 0);
        assert_eq!(item.merge_passes(), 0);

        // Successful publication records the pass:
        assert_eq!(item.record_published_conflict_pass(), 1);
        assert_eq!(item.merge_passes(), 1);
    }

    #[test]
    fn conflict_pass_refund_and_publish_are_scoped_to_conflict_head_and_preserve_prior_budget() {
        let mut item = progress_in(PhaseKind::Approved);
        let head1: CommitId = "1111111111111111111111111111111111111111".parse().unwrap();
        let head2: CommitId = "2222222222222222222222222222222222222222".parse().unwrap();

        // Pass 1 is published and ended:
        item.begin_merge_conflict(head1.clone(), at(10));
        assert_eq!(item.record_published_conflict_pass(), 1);
        item.end_merge_conflict();
        assert_eq!(item.merge_passes(), 1);

        // Pass 2 begins:
        item.begin_merge_conflict(head2.clone(), at(20));
        assert_eq!(item.merge_passes(), 2);

        // First refund restores count to 1:
        assert_eq!(item.refund_conflict_pass(), 1);
        assert_eq!(item.merge_passes(), 1);

        // Second refund for same conflict pass is idempotent and does NOT erase pass 1:
        assert_eq!(item.refund_conflict_pass(), 1);
        assert_eq!(item.merge_passes(), 1);

        // Publication records pass 2:
        assert_eq!(item.record_published_conflict_pass(), 2);
        assert_eq!(item.merge_passes(), 2);

        // Repeated publication recording is idempotent:
        assert_eq!(item.record_published_conflict_pass(), 2);
        assert_eq!(item.merge_passes(), 2);

        // Refund attempt after publication is an idempotent no-op and preserves accounted head:
        assert_eq!(item.refund_conflict_pass(), 2);
        assert_eq!(item.merge_passes(), 2);
        assert_eq!(item.conflict_accounted_head(), Some(&head2));

        // Survives serialization and deserialization:
        let text = serde_json::to_string(&item).unwrap();
        let back: ItemProgress = serde_json::from_str(&text).unwrap();
        assert_eq!(back.merge_passes(), 2);
    }

    #[test]
    fn restart_while_refunded_before_success_retains_refund_and_subsequent_publish_recovers() {
        let mut item = progress_in(PhaseKind::Approved);
        let head: CommitId = "3333333333333333333333333333333333333333".parse().unwrap();
        item.begin_merge_conflict(head.clone(), at(10));
        assert_eq!(item.merge_passes(), 1);

        // Publication fails, refunding pass:
        assert_eq!(item.refund_conflict_pass(), 0);
        assert_eq!(item.merge_passes(), 0);

        // Daemon restarts while refunded:
        let serialized = serde_json::to_string(&item).unwrap();
        let mut recovered: ItemProgress = serde_json::from_str(&serialized).unwrap();
        assert_eq!(recovered.merge_passes(), 0);

        // Later attempt succeeds: publication records and spends pass:
        assert_eq!(recovered.record_published_conflict_pass(), 1);
        assert_eq!(recovered.merge_passes(), 1);
        assert_eq!(recovered.conflict_accounted_head(), Some(&head));

        // Subsequent refund attempt does not erase successfully published pass:
        assert_eq!(recovered.refund_conflict_pass(), 1);
        assert_eq!(recovered.merge_passes(), 1);
    }

    #[test]
    fn contradictory_conflict_pass_heads_are_rejected_on_deserialization() {
        let base = serde_json::to_value(progress_in(PhaseKind::Approved)).unwrap();
        let head1 = "1111111111111111111111111111111111111111";
        let head2 = "2222222222222222222222222222222222222222";
        let head3 = "3333333333333333333333333333333333333333";

        // Case 1: Provisional and accounted:
        let mut c1 = base.clone();
        c1["conflict_provisional_head"] = serde_json::json!(head1);
        c1["conflict_accounted_head"] = serde_json::json!(head2);

        // Case 2: Provisional and refunded:
        let mut c2 = base.clone();
        c2["conflict_provisional_head"] = serde_json::json!(head1);
        c2["conflict_refunded_head"] = serde_json::json!(head2);

        // Case 3: Refunded and accounted:
        let mut c3 = base.clone();
        c3["conflict_refunded_head"] = serde_json::json!(head1);
        c3["conflict_accounted_head"] = serde_json::json!(head2);

        // Case 4: All three:
        let mut c4 = base.clone();
        c4["conflict_provisional_head"] = serde_json::json!(head1);
        c4["conflict_refunded_head"] = serde_json::json!(head2);
        c4["conflict_accounted_head"] = serde_json::json!(head3);

        for case in [c1, c2, c3, c4] {
            let res: Result<ItemProgress, _> = serde_json::from_value(case.clone());
            match res {
                Err(err) => {
                    let msg = err.to_string();
                    assert!(
                        msg.contains("cannot have multiple contradictory conflict pass heads simultaneously"),
                        "unexpected error message: {msg}"
                    );
                }
                Ok(_) => {
                    panic!("contradictory conflict heads must be rejected during deserialization, but parsed: {case}");
                }
            }
        }
    }

    #[test]
    fn asking_human_without_provisional_pass_never_refunds_passes_and_protects_legacy_counts() {
        let mut item = progress_in(PhaseKind::Approved);
        item.spend_merge_pass();
        item.spend_merge_pass();
        assert_eq!(item.merge_passes(), 2);
        let head: CommitId = "4444444444444444444444444444444444444444".parse().unwrap();
        item.set_conflict_approved_head(head);
        item.ask_human_about_merge_conflict(at(50));
        assert!(!item.has_provisional_conflict_pass());

        // Calling refund on an unreserved pass never decrements passes:
        assert_eq!(item.refund_conflict_pass(), 2);
        assert_eq!(item.merge_passes(), 2);

        // Repeated refund attempts remain an idempotent no-op:
        assert_eq!(item.refund_conflict_pass(), 2);
        assert_eq!(item.merge_passes(), 2);

        // Survives round-trip deserialization without losing pass count:
        let serialized = serde_json::to_string(&item).unwrap();
        let mut recovered: ItemProgress = serde_json::from_str(&serialized).unwrap();
        assert_eq!(recovered.merge_passes(), 2);
        assert_eq!(recovered.refund_conflict_pass(), 2);
        assert_eq!(recovered.merge_passes(), 2);
    }

    #[test]
    fn completed_review_passes_counts_both_fix_passes_and_merge_conflict_passes() {
        let mut item = progress_in(PhaseKind::Fixing);
        assert_eq!(item.completed_review_passes(), 1);

        item.spend_merge_pass();
        assert_eq!(item.completed_review_passes(), 2);

        item.spend_merge_pass();
        assert_eq!(item.completed_review_passes(), 3);
    }

    #[test]
    fn reopening_review_after_conflict_advances_completed_review_passes_independently_of_spent_merge_passes() {
        let mut first = progress_in(PhaseKind::Approved);
        first.spend_merge_pass();
        first.spend_merge_pass();
        first.spend_merge_pass();
        first.reopen_review_after_conflict().unwrap();

        let mut second = progress_in(PhaseKind::Approved);
        second.spend_merge_pass();
        second.spend_merge_pass();
        second.spend_merge_pass();
        second.reopen_review_after_conflict().unwrap();
        second.phase = progress_in(PhaseKind::Approved).phase;
        second.reopen_review_after_conflict().unwrap();

        assert_eq!(first.completed_review_passes(), 1);
        assert_eq!(second.completed_review_passes(), 2);
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

    // ---- the reviews of a review pass ---------------------------------------------------------------------------------

    /// A submitted review of `reviewer` (task `n`) with `verdict`.
    fn submitted(reviewer: &str, n: u32, verdict: ReviewVerdict) -> Review {
        review(reviewer, n, ReviewOutcome::Submitted { verdict, at: at(40) })
    }

    /// An item in review with `pass`.
    fn in_review_with(pass: Vec<Review>) -> ItemProgress {
        let mut item = progress_in(PhaseKind::InProgress);
        item.advance(ItemPhase::InReview {
            pr: pr(7),
            reviews: pass,
        })
        .unwrap();
        item
    }

    /// What `advance` says to the item in review `from` moving to approval with `pass`, and the item after it.
    fn approve_with(pass: Vec<Review>) -> (Result<(), AdvanceError>, ItemProgress, ItemProgress) {
        let mut item = progress_in(PhaseKind::InReview);
        let before = item.clone();
        let result = item.advance(ItemPhase::Approved {
            pr: pr(7),
            reviews: pass,
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
        for (pass, cause) in cases {
            let (result, before, after) = approve_with(pass);

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
        for (pass, cause) in cases {
            let mut item = progress_in(PhaseKind::InProgress);
            let before = item.clone();

            let result = item.advance(ItemPhase::InReview {
                pr: pr(7),
                reviews: pass,
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
        for pass in [
            vec![
                submitted("inv-opus", 5, ReviewVerdict::Approve),
                submitted("inv-codex", 6, ReviewVerdict::RequestChanges),
            ],
            vec![
                submitted("inv-opus", 5, ReviewVerdict::RequestChanges),
                review("inv-codex", 6, ReviewOutcome::Pending),
            ],
        ] {
            let mut item = in_review_with(pass);

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
        for (pass, cause) in cases {
            let mut item = in_review_with(pass);
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
    #[test]
    fn legacy_publication_question_requires_complete_identity() {
        let original = serde_json::to_value(progress_in(PhaseKind::InProgress)).unwrap();
        for complete in [false, true] {
            let mut value = original.clone();
            value["publish_question"] = serde_json::to_value(at(100)).unwrap();
            if complete {
                value["publish_question_task"] = serde_json::json!("task-20261111-153001-k3x9");
                value["publish_question_number"] = serde_json::json!(3);
            }
            let item: ItemProgress = serde_json::from_value(value).unwrap();
            assert_eq!(item.publish_question(), complete.then_some(at(100)));
            assert_eq!(
                item.publish_question_number(),
                complete.then_some(NonZeroU32::new(3).unwrap())
            );
        }
    }
    #[test]
    fn publication_question_retains_its_exact_destination_across_restart() {
        let mut item = progress_in(PhaseKind::InProgress);
        let task = "task-20261111-153001-k3x9".parse().unwrap();
        let branch: BranchName = "agents/claude-1/task-1-1.1b".parse().unwrap();
        item.record_publish_question_branch(task, NonZeroU32::new(3).unwrap(), at(100), branch.clone());
        let saved = serde_json::to_value(&item).unwrap();
        let restored: ItemProgress = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.publish_question_branch(), Some(&branch));
        item.clear_publish_question();
        assert_eq!(item.publish_question_branch(), None);
    }
}
