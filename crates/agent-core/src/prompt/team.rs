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
    /// Human instructions for staffing waits that must not use lower-tier agents.
    NoticeTierQuestion,
    /// One item and the capability it is waiting for.
    TierWaiting,
    /// The split proposal correction text.
    SplitCorrection,
    /// What a coordinator is told when the `SPLIT:` it ended with names an item that is split already.
    SplitRefusedAlready,
    /// What a coordinator is told when the `SPLIT:` it ended with names no item of the team.
    SplitRefusedUnknown,
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
    /// The selected member meets the minimum capability.
    TierMember,
    /// The coordinator is the only fitting participant in the crew.
    TierCoordinator,
    /// What a member whose sleep a message ended is told.
    Wake,
    /// What a worker is told when it is given a request of a member.
    Delegation,
    /// What a worker is told, after the delegation, when the request was given to another worker before and that result was
    /// wrong, including a declared FAILED or QUESTIONS ending.
    DelegationAgain,
    /// The message to the coordinator that a member is done with an item.
    MemberFinished,
    /// The message to the coordinator that a member is done, in a team whose task list has no items.
    MemberFinishedPlain,
    /// The message to the coordinator that a member failed.
    MemberFailed,
    /// The message to the coordinator that a member failed while holding an item.
    MemberFailedItem,
    /// A failed member whose run did not report FAILED.
    MemberRunFailed,
    /// A failed item holder whose run did not report FAILED.
    MemberRunFailedItem,
    /// The watchdog ended an inactive member run.
    MemberRunHung,
    /// The account quota ended a member run.
    MemberRunQuota,
    /// The provider rate-limited a member run.
    MemberRunRateLimited,
    /// The human stopped a member run.
    MemberRunStopped,
    /// The status actually reported by a failed member.
    MemberRunReported,
    /// A member run ended without a status.
    MemberRunNoStatus,
    /// A member run exhausted its turn budget.
    MemberRunTurnLimit,
    /// The message to the coordinator that a member ended its turn without a status line.
    MemberNoStatus,
    /// The message to the coordinator that a member ended its turn without a status line while holding an item.
    MemberNoStatusItem,
    /// The message to the lead that the work of a member whose new session went bad too, and which no other agent can
    /// take over, is its own now.
    LeadTakesWork,
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
    /// The message to a member that a worker's result serves its step and was brought to its branch.
    WorkerResultBrought,
    /// The message to a member that a worker's commit does not apply to its branch, with the files in conflict.
    WorkerResultConflict,
    /// The message to a member naming the retry worker and why the previous result or declared ending failed.
    WorkerGivenAgain,
    /// The message to a member that no result serves its step, which it does itself.
    WorkerStepReturned,
    /// The coordinator is told that an observed unmerged close returned an item to its member.
    DeliveryClosed,
    /// The bus tells the team who holds an item now after a busy agent's waiting task was handed over.
    ItemHandedOver,
    /// The notification that a team finished its work.
    NoticeTeamDone,
}

impl TeamPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::DeliveryClosed,
        Self::ItemHandedOver,
        Self::Context,
        Self::SplitRequested,
        Self::NoticeSplitQuestion,
        Self::NoticeTierQuestion,
        Self::TierWaiting,
        Self::SplitCorrection,
        Self::SplitRefusedAlready,
        Self::SplitRefusedUnknown,
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
        Self::TierMember,
        Self::TierCoordinator,
        Self::Wake,
        Self::Delegation,
        Self::DelegationAgain,
        Self::MemberFinished,
        Self::MemberFinishedPlain,
        Self::MemberFailed,
        Self::MemberFailedItem,
        Self::MemberNoStatus,
        Self::MemberRunFailed,
        Self::MemberRunFailedItem,
        Self::MemberRunHung,
        Self::MemberRunQuota,
        Self::MemberRunRateLimited,
        Self::MemberRunStopped,
        Self::MemberRunReported,
        Self::MemberRunNoStatus,
        Self::MemberRunTurnLimit,
        Self::MemberNoStatusItem,
        Self::LeadTakesWork,
        Self::HelperResult,
        Self::HelperFailed,
        Self::WorkerFailed,
        Self::WorkerNoResult,
        Self::WorkerNoStatus,
        Self::WorkerStopped,
        Self::WorkerHung,
        Self::WorkerResultBrought,
        Self::WorkerResultConflict,
        Self::WorkerGivenAgain,
        Self::WorkerStepReturned,
        Self::NoticeTeamDone,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per catalog entry: the entry and its spec stay side by side"
    )]
    pub const fn spec(self) -> PromptSpec {
        match self {
            Self::SplitRequested => split_requested_spec(),
            Self::NoticeSplitQuestion => split_question_spec(),
            Self::NoticeTierQuestion => tier_question_spec(),
            Self::TierWaiting => PromptSpec::new("harness/team/tier-waiting.md", Audience::Model, &["item", "tier"]),
            Self::SplitCorrection => {
                PromptSpec::new("harness/team/split-correction.md", Audience::Model, &["item", "reason"])
            }

            Self::SplitRefusedAlready => refused_already_spec(),
            Self::SplitRefusedUnknown => refused_unknown_spec(),

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
            Self::ItemHandedOver => PromptSpec::new(
                "harness/team/bus-item-handed-over.md",
                Audience::Model,
                &["item", "from", "to", "task", "branch"],
            ),
            Self::Part => part_spec(),
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
            Self::TierMember => tier_spec("harness/team/tier-member.md"),
            Self::TierCoordinator => tier_spec("harness/team/tier-coordinator.md"),
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
            Self::MemberRunFailed => PromptSpec::new(
                "harness/team/bus-member-run-failed.md",
                Audience::Model,
                &["member", "reason", "message"],
            ),
            Self::MemberRunFailedItem => PromptSpec::new(
                "harness/team/bus-member-run-failed-item.md",
                Audience::Model,
                &["member", "item", "branch", "reason", "message"],
            ),
            Self::MemberRunHung => PromptSpec::new("harness/team/member-run-hung.md", Audience::Model, &[]),
            Self::MemberRunQuota => PromptSpec::new("harness/team/member-run-quota.md", Audience::Model, &[]),
            Self::MemberRunRateLimited => {
                PromptSpec::new("harness/team/member-run-rate-limited.md", Audience::Model, &[])
            }
            Self::MemberRunStopped => PromptSpec::new("harness/team/member-run-stopped.md", Audience::Model, &[]),
            Self::MemberRunReported => {
                PromptSpec::new("harness/team/member-run-reported.md", Audience::Model, &["outcome"])
            }
            Self::MemberRunNoStatus => PromptSpec::new("harness/team/member-run-no-status.md", Audience::Model, &[]),
            Self::MemberRunTurnLimit => PromptSpec::new("harness/team/member-run-turn-limit.md", Audience::Model, &[]),
            Self::MemberNoStatus => {
                PromptSpec::new("harness/team/bus-member-no-status.md", Audience::Model, &["member"])
            }
            Self::MemberNoStatusItem => PromptSpec::new(
                "harness/team/bus-member-no-status-item.md",
                Audience::Model,
                &["member", "item", "branch"],
            ),
            Self::LeadTakesWork => lead_takes_work_spec(),
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
            Self::WorkerResultBrought => brought_result_spec(),
            Self::WorkerResultConflict => conflict_result_spec(),
            Self::DelegationAgain => PromptSpec::new(
                "harness/team/delegation-again.md",
                Audience::Model,
                &["previous_worker", "reason"],
            ),
            Self::WorkerGivenAgain => given_again_spec(),
            Self::WorkerStepReturned => step_returned_spec(),
            Self::NoticeTeamDone => PromptSpec::new(
                "harness/team/notice-team-done.md",
                Audience::Human,
                &["team", "repo", "items"],
            ),
        }
    }
}

/// The text of a result a worker's check let through and that the harness brought to the requester's branch.
const fn brought_result_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/worker-result-brought.md",
        Audience::Model,
        &["worker", "step", "commit", "files", "tail"],
    )
}

/// The text of a worker's commit that does not apply to the requester's branch.
const fn conflict_result_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/worker-result-conflict.md",
        Audience::Model,
        &["worker", "step", "commit", "files"],
    )
}

/// The text of a request given once more to another worker, with the reason the first result did not serve.
const fn given_again_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/worker-given-again.md",
        Audience::Model,
        &["worker", "request_id", "reason", "next"],
    )
}

/// The text of a step returned to the requester with every reason its results did not serve.
const fn step_returned_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/worker-step-returned.md",
        Audience::Model,
        &["request_id", "step", "reasons"],
    )
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

/// The message to the lead that the work of a member is its own now.
const fn lead_takes_work_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/bus-lead-takes-work.md",
        Audience::Model,
        &["member", "task", "branch", "worktree"],
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

/// The capability explanation has the same inputs for a member and coordinator.
const fn tier_spec(path: &'static str) -> PromptSpec {
    PromptSpec::new(path, Audience::Model, &["required", "agent", "tier"])
}

/// The question about staffing without weakening a capability requirement.
const fn tier_question_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/notice-tier-question.md",
        Audience::Human,
        &["task", "waiting"],
    )
}

/// The independently deliverable part and its previously merged dependencies.
const fn part_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/team/part.md",
        Audience::Model,
        &["part", "title", "delivers", "checks", "previous"],
    )
}

/// The text that tells a coordinator its `SPLIT:` line names an item that is split already.
const fn refused_already_spec() -> PromptSpec {
    PromptSpec::new("harness/team/split-refused-already.md", Audience::Model, &["item"])
}

/// The text that tells a coordinator its `SPLIT:` line names no item of the team.
const fn refused_unknown_spec() -> PromptSpec {
    PromptSpec::new("harness/team/split-refused-unknown.md", Audience::Model, &["text"])
}
