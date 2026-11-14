//! What happens in a round beyond what its runs do: gates, checks, the pull request and its reviews, questions,
//! controls, notifications, and the ends of a round and a team.
//!
//! An [`EventRecord`] is one such happening with its time, team and rounds. Its payload, a [`HistoryEvent`], is kept as
//! JSON in the ledger of the history (`docs/history.md`, "Events"), so a variant is a format that old data must read.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::decide::round::AskReason;
use crate::ids::{BranchName, InvestigatorName, RoundId, TaskId, TeamId};
use crate::kinds::{Keyword, UnknownKeyword};
use crate::protocol::{Client, ControlAction, Target};
use crate::state::{AnsweredVia, CommitId, GateRecord, PrNumber, RoundEnd, Timestamp};
use crate::status::ReviewVerdict;

/// The keyword of a [`HistoryEvent`], as the ledger keeps it in its `kind` column.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<EventKind>)]
pub enum EventKind {
    /// [`HistoryEvent::Gate`].
    Gate,
    /// [`HistoryEvent::Checked`].
    Checked,
    /// [`HistoryEvent::PullOpened`].
    PullOpened,
    /// [`HistoryEvent::PullPushed`].
    PullPushed,
    /// [`HistoryEvent::PullReady`].
    PullReady,
    /// [`HistoryEvent::ReviewAsked`].
    ReviewAsked,
    /// [`HistoryEvent::ReviewSubmitted`].
    ReviewSubmitted,
    /// [`HistoryEvent::FixPassStarted`].
    FixPassStarted,
    /// [`HistoryEvent::PullApproved`].
    PullApproved,
    /// [`HistoryEvent::PullMerged`].
    PullMerged,
    /// [`HistoryEvent::PullHandedOver`].
    PullHandedOver,
    /// [`HistoryEvent::PullClosed`].
    PullClosed,
    /// [`HistoryEvent::QuestionAsked`].
    QuestionAsked,
    /// [`HistoryEvent::QuestionAnswered`].
    QuestionAnswered,
    /// [`HistoryEvent::Controlled`].
    Controlled,
    /// [`HistoryEvent::Notified`].
    Notified,
    /// [`HistoryEvent::RoundClosed`].
    RoundClosed,
    /// [`HistoryEvent::TeamEnded`].
    TeamEnded,
}

impl Keyword for EventKind {
    const WHAT: &'static str = "kind of event";
}

/// What followed the checks before a review found issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckOutcome {
    /// The round went back to the team to fix them.
    SentBack {
        /// The pass of the checks that starts, counted from 1.
        pass: NonZeroU32,
        /// How many passes the checks allow before they ask the human.
        of: u32,
    },
    /// The human was asked what the issues are worth.
    AskedHuman {
        /// Why they were asked.
        reason: AskReason,
        /// The number of the questions that name the issues.
        question: NonZeroU32,
    },
    /// The human had allowed every one of them: the reviews go on.
    Allowed,
}

/// One happening of a round or a team. The variants are the payload of the ledger: a name or a field of one is a format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryEvent {
    /// The gate ran for a task; the time of the record is when it ended.
    Gate {
        /// The task whose worktree the gate ran in.
        task: TaskId,
        /// When the gate began to run, after any wait for its turn.
        started_at: Timestamp,
        /// What the daemon recorded about it: the head, the tree, the outcome and the file of its output.
        record: GateRecord,
    },
    /// The checks before a review found issues.
    Checked {
        /// The pull request, when there is one.
        pr: Option<PrNumber>,
        /// The head that was checked.
        head: CommitId,
        /// Each issue, in a line.
        issues: Vec<String>,
        /// What followed.
        outcome: CheckOutcome,
    },
    /// A pull request was opened.
    PullOpened {
        /// Its number.
        number: PrNumber,
        /// Its branch.
        branch: BranchName,
        /// The head it was opened at.
        head: CommitId,
        /// Its title.
        title: String,
        /// Its description, whole.
        body: String,
    },
    /// The branch of a pull request was pushed.
    PullPushed {
        /// The pull request.
        pr: PrNumber,
        /// The new head.
        head: CommitId,
    },
    /// A pull request was marked ready for review.
    PullReady {
        /// The pull request.
        pr: PrNumber,
    },
    /// A reviewer was asked for a review.
    ReviewAsked {
        /// The pull request.
        pr: PrNumber,
        /// The review pass: `0` for the first review, then one more for each fix pass.
        pass: u32,
        /// The head to review.
        head: CommitId,
        /// The reviewer.
        reviewer: InvestigatorName,
        /// The reviewer's task.
        task: TaskId,
    },
    /// A reviewer submitted a review.
    ReviewSubmitted {
        /// The pull request.
        pr: PrNumber,
        /// The review pass, as in [`HistoryEvent::ReviewAsked`].
        pass: u32,
        /// The reviewer.
        reviewer: InvestigatorName,
        /// The reviewer's task.
        task: TaskId,
        /// The verdict.
        verdict: ReviewVerdict,
        /// The report, whole.
        report: String,
    },
    /// A fix pass started: the reviews asked for changes and the team gets their findings.
    FixPassStarted {
        /// The pull request.
        pr: PrNumber,
        /// The fix pass, counted from 1.
        pass: NonZeroU32,
        /// How many fix passes the item has before the human is asked.
        of: u32,
        /// The findings sent to the team, whole.
        findings: String,
    },
    /// Both reviews approved.
    PullApproved {
        /// The pull request.
        pr: PrNumber,
    },
    /// A pull request was merged.
    PullMerged {
        /// The pull request.
        pr: PrNumber,
        /// The merge commit; `None` when the remote did not say.
        commit: Option<CommitId>,
    },
    /// An approved pull request was handed to the human (`--no-merge`).
    PullHandedOver {
        /// The pull request.
        pr: PrNumber,
    },
    /// A pull request was closed without a merge.
    PullClosed {
        /// The pull request.
        pr: PrNumber,
    },
    /// A task asked the human questions.
    QuestionAsked {
        /// The task.
        task: TaskId,
        /// The number of the questions, counted from 1 for each task.
        number: NonZeroU32,
        /// The questions file, whole.
        text: String,
    },
    /// The human answered questions.
    QuestionAnswered {
        /// The task.
        task: TaskId,
        /// The number of the questions.
        number: NonZeroU32,
        /// The questions file with the verdicts, whole.
        text: String,
        /// The channel the answer came through.
        via: AnsweredVia,
    },
    /// The human controlled a task or a team.
    Controlled {
        /// Which control.
        action: ControlAction,
        /// What it was aimed at.
        target: Target,
        /// Who sent it.
        by: Client,
        /// The tasks it reached.
        tasks: Vec<TaskId>,
    },
    /// A notification was sent to the human.
    Notified {
        /// The task it is about, when it is about one.
        task: Option<TaskId>,
        /// Its title.
        title: String,
        /// Its text.
        body: String,
    },
    /// A round closed.
    RoundClosed {
        /// The round.
        round: RoundId,
        /// How it ended.
        end: RoundEnd,
    },
    /// A team ended.
    TeamEnded {},
}

/// An event with when it happened and whom it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    /// When it happened.
    pub at: Timestamp,
    /// The team.
    pub team: TeamId,
    /// The rounds it belongs to, oldest first: empty for what is not about a round (a team ending).
    pub rounds: Vec<RoundId>,
    /// What happened.
    pub event: HistoryEvent,
}

impl HistoryEvent {
    /// The keyword of the event.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        EventKind::Gate
    }

    /// What makes the event the same event when it is recorded twice: a pull request opened, a head pushed, a review
    /// asked. `None` for what can happen again as often as it likes (a gate, a control, a notification).
    #[must_use]
    pub fn key(&self) -> Option<String> {
        None
    }

    /// Every free text of the event, with the name of its field, to redact and to cut. The name is what a [`Cut`]
    /// lists.
    ///
    /// [`Cut`]: super::Cut
    pub fn texts_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        Vec::new()
    }

    /// The identities in the event — the strings a reader must find as they were written — which a credential must not
    /// occur in: a record that has one is not written.
    #[must_use]
    pub fn identities(&self) -> Vec<&str> {
        Vec::new()
    }
}
