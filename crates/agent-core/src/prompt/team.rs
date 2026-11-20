//! The texts of a team's life: what a member is told about its team, what wakes it, what a worker is asked, the messages
//! the harness puts on the bus for the coordinator, and the notification that a team is finished
//! (`prompts/harness/team/<name>.md`).
//!
//! They are a third catalog beside [`PromptId`](super::PromptId) and [`HarnessPrompt`](super::HarnessPrompt), with the
//! same shape — a file under [`PROMPTS_DIR`](super::PROMPTS_DIR), an [`Audience`] and the variables its text uses — and
//! its own directory, which the integration test `tests/harness_team_prompts.rs` checks against the entries.

use super::{Audience, PromptSpec};

/// Every text of a team's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TeamPrompt {
    /// What the first run of a team's member is told about the team: who is in it, its branch, how it talks to the others.
    Context,
    /// A member asks its coordinator to submit a split proposal.
    SplitRequested,
    /// The split proposal notice-question text.
    NoticeSplitQuestion,
    /// The split proposal correction text.
    SplitCorrection,
    /// The split proposal limit text.
    SplitLimit,
    /// The split proposal count text.
    SplitCount,
    /// The split proposal deliverable text.
    SplitDeliverable,
    /// The split proposal title text.
    SplitTitle,
    /// The split proposal duplicate text.
    SplitDuplicate,
    /// The split proposal unknown text.
    SplitUnknown,
    /// The split proposal first text.
    SplitFirst,
    /// The split proposal cycle text.
    SplitCycle,
    /// The split proposal checks text.
    SplitChecks,
    /// The split proposal unreadable text.
    SplitUnreadable,
    /// The independent deliverable and checks of the active split part.
    Part,
    /// What a member that already has a session is told when it is given its next item.
    Assignment,
    /// What a member whose sleep a message ended is told.
    Wake,
    /// What a worker is told when it is given a request of a member.
    Delegation,
    /// The message to the coordinator that a member is done with an item.
    MemberFinished,
    /// The message to the coordinator that a member is done, in a team whose task list has no items.
    MemberFinishedPlain,
    /// The message to the coordinator that a member failed.
    MemberFailed,
    /// The message to the coordinator that a member failed while holding an item.
    MemberFailedItem,
    /// The message to the coordinator that a member ended its turn without a status line.
    MemberNoStatus,
    /// The message to the coordinator that a member ended its turn without a status line while holding an item.
    MemberNoStatusItem,
    /// The message to a member that the worker finished its request.
    HelperResult,
    /// The message to a member that the worker failed its request.
    HelperFailed,
    /// How a worker's run that failed a request ended: with `STATUS: FAILED`.
    WorkerFailed,
    /// How a worker's run that failed a request ended: with nothing a requester can use, or questions that cannot be
    /// used.
    WorkerNoResult,
    /// How a worker's run that failed a request ended: without a status line, at a provider's limit it cannot wait for,
    /// or it could not run.
    WorkerNoStatus,
    /// How a worker's run that failed a request ended: the human stopped it.
    WorkerStopped,
    /// How a worker's run that failed a request ended: the watchdog gave up on it.
    WorkerHung,
    /// The coordinator is told that an observed unmerged close returned an item to its member.
    DeliveryClosed,
    /// The notification that a team finished its work.
    NoticeTeamDone,
}

impl TeamPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::DeliveryClosed,
        Self::Context,
        Self::SplitRequested,
        Self::NoticeSplitQuestion,
        Self::SplitCorrection,
        Self::SplitLimit,
        Self::SplitCount,
        Self::SplitDeliverable,
        Self::SplitTitle,
        Self::SplitDuplicate,
        Self::SplitUnknown,
        Self::SplitFirst,
        Self::SplitCycle,
        Self::SplitChecks,
        Self::SplitUnreadable,
        Self::Part,
        Self::Assignment,
        Self::Wake,
        Self::Delegation,
        Self::MemberFinished,
        Self::MemberFinishedPlain,
        Self::MemberFailed,
        Self::MemberFailedItem,
        Self::MemberNoStatus,
        Self::MemberNoStatusItem,
        Self::HelperResult,
        Self::HelperFailed,
        Self::WorkerFailed,
        Self::WorkerNoResult,
        Self::WorkerNoStatus,
        Self::WorkerStopped,
        Self::WorkerHung,
        Self::NoticeTeamDone,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        match self {
            Self::SplitRequested => split_requested_spec(),
            Self::NoticeSplitQuestion => split_question_spec(),
            Self::SplitCorrection => {
                PromptSpec::new("harness/team/split-correction.md", Audience::Model, &["item", "reason"])
            }

            Self::SplitLimit => PromptSpec::new(
                "harness/team/split-limit.md",
                Audience::Model,
                &["part", "amount", "unit", "limit"],
            ),

            Self::SplitCount => PromptSpec::new("harness/team/split-count.md", Audience::Model, &["count"]),

            Self::SplitDeliverable => PromptSpec::new("harness/team/split-deliverable.md", Audience::Model, &["part"]),

            Self::SplitTitle => PromptSpec::new("harness/team/split-title.md", Audience::Model, &["part"]),

            Self::SplitDuplicate => PromptSpec::new(
                "harness/team/split-duplicate.md",
                Audience::Model,
                &["part", "dependency"],
            ),

            Self::SplitUnknown => PromptSpec::new(
                "harness/team/split-unknown.md",
                Audience::Model,
                &["part", "dependency"],
            ),

            Self::SplitFirst => PromptSpec::new("harness/team/split-first.md", Audience::Model, &["part"]),

            Self::SplitCycle => PromptSpec::new("harness/team/split-cycle.md", Audience::Model, &["part"]),

            Self::SplitChecks => PromptSpec::new("harness/team/split-checks.md", Audience::Model, &["part"]),

            Self::SplitUnreadable => PromptSpec::new("harness/team/split-unreadable.md", Audience::Model, &["part"]),

            Self::DeliveryClosed => {
                PromptSpec::new("harness/team/bus-delivery-closed.md", Audience::Model, &["item", "pr"])
            }
            Self::Part => PromptSpec::new(
                "harness/team/part.md",
                Audience::Model,
                &["part", "title", "delivers", "checks", "previous"],
            ),
            Self::Context => PromptSpec::new(
                "harness/team/context.md",
                Audience::Model,
                &["team", "coordinator", "members", "workers", "branch", "base"],
            ),
            Self::Assignment => PromptSpec::new(
                "harness/team/assignment.md",
                Audience::Model,
                &["item_id", "branch", "base"],
            ),
            Self::Wake => PromptSpec::new("harness/team/wake.md", Audience::Model, &["team"]),
            Self::Delegation => delegation_spec(),
            Self::MemberFinished => PromptSpec::new(
                "harness/team/bus-member-finished.md",
                Audience::Model,
                &["member", "item", "branch"],
            ),
            Self::MemberFinishedPlain => PromptSpec::new(
                "harness/team/bus-member-finished-plain.md",
                Audience::Model,
                &["member", "branch"],
            ),
            Self::MemberFailed => PromptSpec::new(
                "harness/team/bus-member-failed.md",
                Audience::Model,
                &["member", "message"],
            ),
            Self::MemberFailedItem => PromptSpec::new(
                "harness/team/bus-member-failed-item.md",
                Audience::Model,
                &["member", "item", "branch", "message"],
            ),
            Self::MemberNoStatus => {
                PromptSpec::new("harness/team/bus-member-no-status.md", Audience::Model, &["member"])
            }
            Self::MemberNoStatusItem => PromptSpec::new(
                "harness/team/bus-member-no-status-item.md",
                Audience::Model,
                &["member", "item", "branch"],
            ),
            Self::HelperResult => PromptSpec::new(
                "harness/team/bus-helper-result.md",
                Audience::Model,
                &["worker", "request_id", "summary"],
            ),
            Self::HelperFailed => PromptSpec::new(
                "harness/team/bus-helper-failed.md",
                Audience::Model,
                &["worker", "request_id", "reason"],
            ),
            Self::WorkerFailed => PromptSpec::new("harness/team/worker-failed.md", Audience::Model, &[]),
            Self::WorkerNoResult => PromptSpec::new("harness/team/worker-no-result.md", Audience::Model, &[]),
            Self::WorkerNoStatus => PromptSpec::new("harness/team/worker-no-status.md", Audience::Model, &[]),
            Self::WorkerStopped => PromptSpec::new("harness/team/worker-stopped.md", Audience::Model, &[]),
            Self::WorkerHung => PromptSpec::new("harness/team/worker-hung.md", Audience::Model, &[]),
            Self::NoticeTeamDone => PromptSpec::new(
                "harness/team/notice-team-done.md",
                Audience::Human,
                &["team", "repo", "items"],
            ),
        }
    }
}

/// The delegation form and its complete set of model inputs.
const fn delegation_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/delegation.md",
        Audience::Model,
        &[
            "requester",
            "request_id",
            "requester_worktree",
            "requester_branch",
            "request",
            "context",
        ],
    )
}

/// Catalog variables for the member-to-coordinator request.
const fn split_requested_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/split-requested.md",
        Audience::Model,
        &["member", "item", "task"],
    )
}

/// Catalog variables for the harness-created human question.
const fn split_question_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/notice-split-question.md",
        Audience::Human,
        &["task", "item", "reason", "proposal", "example"],
    )
}
