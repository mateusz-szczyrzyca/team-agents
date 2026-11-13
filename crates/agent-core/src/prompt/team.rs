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
    /// The message to the coordinator that a member ended its turn without a status line.
    MemberNoStatus,
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
    /// The notification that a team finished its work.
    NoticeTeamDone,
}

impl TeamPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::Context,
        Self::Assignment,
        Self::Wake,
        Self::Delegation,
        Self::MemberFinished,
        Self::MemberFinishedPlain,
        Self::MemberFailed,
        Self::MemberNoStatus,
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
            Self::Delegation => PromptSpec::new(
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
            ),
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
            Self::MemberNoStatus => {
                PromptSpec::new("harness/team/bus-member-no-status.md", Audience::Model, &["member"])
            }
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
