//! The vocabulary of a checkpoint: the point of a round from which a new session, another agent or a new team can go on.
//!
//! A checkpoint is the history record [`HistoryEvent::Checkpoint`](super::HistoryEvent::Checkpoint). Which step of a
//! round leaves one is decided here, by [`HistoryEvent::checkpoint_cause`]; taking it (the snapshot of the worktree and the
//! write) is the daemon's work.

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr, VariantArray, VariantNames};

use super::HistoryEvent;
use crate::kinds::{Keyword, UnknownKeyword};

mod job;
mod state;

pub use job::{CheckResult, JobWork, ReviewedItem, investigation_meant, job_content, request_meant, review_meant};
pub use state::{CommitLine, Counts, FindingLine, LIST_LIMIT, Next, StateInput, checkpoint_content};

/// What a checkpoint is for: the kind of work whose state it keeps.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
    VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<CheckpointJob>)]
pub enum CheckpointJob {
    /// An item of a task list: a lead, a member, a solo agent or a coordinator working on its round.
    Item,
    /// A request a worker serves for a member.
    WorkerRequest,
    /// A review of a pull request.
    Review,
    /// An investigation.
    Investigation,
}

impl Default for CheckpointJob {
    /// The job of a record that names none: those written before the other jobs had checkpoints are all items.
    fn default() -> Self {
        Self::Review
    }
}

impl Keyword for CheckpointJob {
    const WHAT: &'static str = "kind of checkpoint job";
}

/// The step of a round that a checkpoint was taken right after.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
    VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<CheckpointCause>)]
pub enum CheckpointCause {
    /// A round opened: its branch was made.
    RoundOpened,
    /// A new local commit in the worktree of a lead or a member, pushed or not.
    Commit,
    /// A pull request was opened, as a draft or not.
    PullOpened,
    /// The branch of a pull request was pushed.
    PullPushed,
    /// A pull request was marked ready for review.
    PullReady,
    /// The checks before a review ran.
    Checked,
    /// The harness decided on a split proposal.
    SplitDecided,
    /// A reviewer submitted its review.
    ReviewSubmitted,
    /// A fix pass started.
    FixPassStarted,
    /// A finding was explained, accepted, rejected or raised.
    FindingAnswered,
    /// A verifier confirmed a finding or did not.
    FindingJudged,
    /// The round waits for the human after its branch was not published.
    WaitsForHuman,
    /// A task asked the human questions.
    QuestionAsked,
    /// The human answered questions.
    QuestionAnswered,
    /// Both reviews approved the pull request.
    PullApproved,
    /// A merge was refused for a conflict.
    MergeRefused,
    /// A pull request was merged.
    PullMerged,
    /// An approved pull request was handed to the human.
    PullHandedOver,
    /// A pull request was closed without a merge.
    PullClosed,
    /// A run of a lead or a member of the round ended.
    RunEnded,
    /// The round limit cut the round.
    RoundOverdue,
    /// The round got another holder: a hand-over, a renewal or a new session.
    HolderChanged,
    /// The daemon started and found the worktree of a task of an open round changed since its last checkpoint.
    DaemonStarted,
    /// A worker started serving a request.
    RequestStarted,
    /// The check of a worker's request ran.
    RequestChecked,
    /// A reviewer started its review.
    ReviewStarted,
    /// The reviewer's findings file has findings that the last checkpoint did not have.
    FindingsWritten,
    /// An investigator started an investigation.
    InvestigationStarted,
    /// An investigator answered the question of its investigation.
    InvestigationAnswered,
}

impl Keyword for CheckpointCause {
    const WHAT: &'static str = "cause of a checkpoint";
}

/// What a checkpoint says about the work, in three parts: what is to be done in general, what is done in relation to
/// that, and what is left of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointContent {
    /// What is to be done in general.
    pub meant: String,
    /// What is done already, in relation to [`CheckpointContent::meant`].
    pub done: String,
    /// What is left of [`CheckpointContent::meant`].
    pub left: String,
}

impl HistoryEvent {
    /// The checkpoint that follows the record of this event, or `None` for an event after which none is taken.
    ///
    /// The `match` names every variant: a new one must be placed here, in a step that leaves a checkpoint or in one that
    /// does not. The causes without a history record (a round opened, a commit, a run ended, ...) are not here.
    #[must_use]
    pub const fn checkpoint_cause(&self) -> Option<CheckpointCause> {
        match self {
            Self::PullOpened { .. } => Some(CheckpointCause::PullOpened),
            Self::PullPushed { .. } => Some(CheckpointCause::PullPushed),
            Self::PullReady { .. } => Some(CheckpointCause::PullReady),
            Self::Checked { .. } => Some(CheckpointCause::Checked),
            Self::SplitDecision { .. } => Some(CheckpointCause::SplitDecided),
            Self::ReviewSubmitted { .. } => Some(CheckpointCause::ReviewSubmitted),
            Self::FixPassStarted { .. } => Some(CheckpointCause::FixPassStarted),
            Self::FindingExplained { .. }
            | Self::FindingAccepted { .. }
            | Self::FindingRejected { .. }
            | Self::FindingRaised { .. } => Some(CheckpointCause::FindingAnswered),
            Self::FindingConfirmed { .. } | Self::FindingNotConfirmed { .. } => Some(CheckpointCause::FindingJudged),
            Self::QuestionAsked { .. } => Some(CheckpointCause::QuestionAsked),
            Self::QuestionAnswered { .. } => Some(CheckpointCause::QuestionAnswered),
            Self::PullApproved { .. } => Some(CheckpointCause::PullApproved),
            Self::PullMerged { .. } => Some(CheckpointCause::PullMerged),
            Self::PullHandedOver { .. } => Some(CheckpointCause::PullHandedOver),
            Self::PullClosed { .. } => Some(CheckpointCause::PullClosed),
            // No checkpoint: the gate and the asking of a review and of a second opinion are not a step the work can be
            // resumed from (the review's own record is), and these are no step of the round's work at all.
            Self::Gate { .. }
            | Self::ReviewAsked { .. }
            | Self::SecondOpinionAsked { .. }
            | Self::SecondOpinionEnded { .. }
            | Self::Controlled { .. }
            | Self::SerenaReminded { .. }
            | Self::Notified { .. }
            | Self::RoundClosed { .. }
            | Self::TeamEnded {}
            | Self::Checkpoint { .. }
            | Self::Lost { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
