//! The texts the harness itself composes: the run prompts and the notifications (`prompts/harness/<module>/<name>.md`).
//!
//! They are a second catalog beside [`PromptId`](super::PromptId): that one lists the rule files that a layout of the
//! repository pins, this one the texts of the daemon, which grow with its features. An entry has the same shape — a file
//! under [`PROMPTS_DIR`](super::PROMPTS_DIR), an [`Audience`] and the variables its text uses — and the integration test
//! `tests/harness_prompt_catalog.rs` checks every entry against its file.

use super::{Audience, PromptSpec};

/// Every text the harness composes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HarnessPrompt {
    /// The working context every run starts with: who the agent is, where it works, how a turn ends and how it asks the human.
    Context,
    /// The role of the only agent of a task.
    RoleSolo,
    /// The role of the coordinator of a team.
    RoleCoordinator,
    /// The role of a member of a team.
    RoleMember,
    /// The role of a worker.
    RoleWorker,
    /// The role of a reviewer of a pull request.
    RoleReviewer,
    /// The brief of the task list: the text before its first feature.
    Brief,
    /// The item an agent is given: its number, title and description.
    Item,
    /// The expected result of an item, for an item that has one.
    ItemExpected,
    /// The checks of an item, for an item that has them.
    ItemChecks,
    /// What comes before the items of a team, for its coordinator.
    TeamItems,
    /// An item of the team a coordinator leads: its number, title, description and who has it.
    TeamItem,
    /// The last checkpoint of an agent, for a run that has one.
    Checkpoint,
    /// What a run resumed after the watchdog killed it is told.
    ResumeHang,
    /// What a run resumed after a restart of the harness is told.
    ResumeRestart,
    /// What a run resumed after a provider's limit is told.
    ResumeLimit,
    /// Continue a session after its turn budget ran out.
    ResumeTurnLimit,
    /// Continue a session after the run ended without a STATUS: line.
    ResumeNoStatus,
    /// The briefing of a task started in a new session after its runs went bad.
    Briefing,
    /// What the runs of a member's task left, as its lead is told when the work is its own: the briefing without the
    /// task's prompt.
    LeadBriefing,
    /// The note of a briefing whose list of unread messages leaves older ones out.
    BriefingMessagesLeftOut,
    /// Tell the human that consecutive turn-limit resumes are exhausted.
    NoticeTurnLimitExhausted,
    /// Tell the human that a turn-limited run had no session to resume.
    NoticeTurnLimitNoSession,
    /// The short prompt that sends the agent to a file when its task is too long for a command line.
    PromptFile,
    /// The notification that an agent has questions.
    NoticeQuestions,
    /// The notification that a run ended with questions that cannot be read.
    NoticeQuestionsMissing,
    /// The notification that a task was marked failed after the watchdog's last strike.
    NoticeHung,
    /// The notification that a task is paused for a provider's limit.
    NoticePaused,
    /// The notification that an agent reported `STATUS: FAILED`.
    NoticeFailed,
    /// The notification that a run ended without a status line, with an exit code.
    NoticeNoStatus,
    /// The notification that a run a signal ended had no status line.
    NoticeNoStatusSignal,
    /// The notification that a run could not be started.
    NoticeStartFailed,
    /// The notification that a run that started could not be carried out to its end.
    NoticeRunFailed,
    /// The notification that free disk space is below `low_free_gb`.
    NoticeLowDisk,
    /// The notification that open rounds wait for disk space.
    NoticeDiskWait,
    /// How a solo agent, a member or a coordinator keeps the plan of its task in `.agent/plan.md`, and its form.
    Plan,
    /// What a run sent back because it ended done without a plan is told.
    PlanMissing,
    /// What a run sent back because its plan does not parse is told.
    PlanUnreadable,
    /// What a run sent back because steps of its plan are still to do or in hand is told.
    PlanUnfinished,
    /// What a run sent back because its plan lost a step of the version before is told.
    PlanLostStep,
    /// The questions file the harness writes for the human when a task was sent back for its plan as often as
    /// `[plan] bounces` allows.
    NoticePlanQuestion,
}

impl HarnessPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::Context,
        Self::RoleSolo,
        Self::RoleCoordinator,
        Self::RoleMember,
        Self::RoleWorker,
        Self::RoleReviewer,
        Self::Brief,
        Self::Item,
        Self::ItemExpected,
        Self::ItemChecks,
        Self::TeamItems,
        Self::TeamItem,
        Self::Checkpoint,
        Self::ResumeHang,
        Self::ResumeRestart,
        Self::ResumeLimit,
        Self::ResumeTurnLimit,
        Self::ResumeNoStatus,
        Self::Briefing,
        Self::LeadBriefing,
        Self::BriefingMessagesLeftOut,
        Self::NoticeTurnLimitExhausted,
        Self::NoticeTurnLimitNoSession,
        Self::PromptFile,
        Self::NoticeQuestions,
        Self::NoticeQuestionsMissing,
        Self::NoticeHung,
        Self::NoticePaused,
        Self::NoticeFailed,
        Self::NoticeNoStatus,
        Self::NoticeNoStatusSignal,
        Self::NoticeStartFailed,
        Self::NoticeRunFailed,
        Self::NoticeLowDisk,
        Self::NoticeDiskWait,
        Self::Plan,
        Self::PlanMissing,
        Self::PlanUnreadable,
        Self::PlanUnfinished,
        Self::PlanLostStep,
        Self::NoticePlanQuestion,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive arm per catalog text keeps the paths and variables auditable together"
    )]
    pub const fn spec(self) -> PromptSpec {
        match self {
            Self::Context => PromptSpec::new(
                "harness/run/context.md",
                Audience::Model,
                &["agent", "task", "repo", "worktree", "questions_file", "number"],
            ),
            Self::RoleSolo => PromptSpec::new("harness/run/role-solo.md", Audience::Model, &[]),
            Self::RoleCoordinator => PromptSpec::new("harness/run/role-coordinator.md", Audience::Model, &[]),
            Self::RoleMember => PromptSpec::new("harness/run/role-member.md", Audience::Model, &[]),
            Self::RoleWorker => PromptSpec::new("harness/run/role-worker.md", Audience::Model, &[]),
            Self::RoleReviewer => PromptSpec::new("harness/run/role-reviewer.md", Audience::Model, &[]),
            Self::Brief => PromptSpec::new("harness/run/brief.md", Audience::Model, &["brief"]),
            Self::Item => PromptSpec::new(
                "harness/run/item.md",
                Audience::Model,
                &["item_id", "item_title", "item_description"],
            ),
            Self::ItemExpected => PromptSpec::new("harness/run/item-expected.md", Audience::Model, &["expected"]),
            Self::ItemChecks => PromptSpec::new("harness/run/item-checks.md", Audience::Model, &["checks"]),
            Self::TeamItems => PromptSpec::new("harness/run/team-items.md", Audience::Model, &[]),
            Self::TeamItem => PromptSpec::new(
                "harness/run/team-item.md",
                Audience::Model,
                &["item_id", "item_title", "item_description", "who"],
            ),
            Self::Checkpoint => PromptSpec::new("harness/run/checkpoint.md", Audience::Model, &["checkpoint"]),
            Self::ResumeHang => PromptSpec::new("harness/run/resume-hang.md", Audience::Model, &["minutes"]),
            Self::ResumeRestart => PromptSpec::new("harness/run/resume-restart.md", Audience::Model, &[]),
            Self::ResumeTurnLimit => PromptSpec::new("harness/run/resume-turn-limit.md", Audience::Model, &[]),
            Self::ResumeNoStatus => PromptSpec::new("harness/run/resume-no-status.md", Audience::Model, &[]),
            Self::Briefing => PromptSpec::new(
                "harness/run/briefing.md",
                Audience::Model,
                &[
                    "reason",
                    "task",
                    "prompt",
                    "base",
                    "branch",
                    "commits",
                    "uncommitted",
                    "snapshot",
                    "plan",
                    "left_out",
                    "messages",
                ],
            ),
            Self::LeadBriefing => lead_briefing_spec(),
            Self::BriefingMessagesLeftOut => {
                PromptSpec::new("harness/run/briefing-messages-left-out.md", Audience::Model, &[])
            }
            Self::NoticeTurnLimitExhausted => PromptSpec::new(
                "harness/run/notice-turn-limit-exhausted.md",
                Audience::Human,
                &["agent", "task", "count", "limit"],
            ),
            Self::NoticeTurnLimitNoSession => PromptSpec::new(
                "harness/run/notice-turn-limit-no-session.md",
                Audience::Human,
                &["agent", "task"],
            ),
            Self::ResumeLimit => PromptSpec::new("harness/run/resume-limit.md", Audience::Model, &[]),
            Self::PromptFile => PromptSpec::new("harness/run/prompt-file.md", Audience::Model, &["path"]),
            Self::NoticeQuestions => PromptSpec::new(
                "harness/run/notice-questions.md",
                Audience::Human,
                &["agent", "task", "number", "file"],
            ),
            Self::NoticeQuestionsMissing => PromptSpec::new(
                "harness/run/notice-questions-missing.md",
                Audience::Human,
                &["agent", "task", "file", "reason"],
            ),
            Self::NoticeHung => PromptSpec::new(
                "harness/run/notice-hung.md",
                Audience::Human,
                &["agent", "task", "minutes", "strikes"],
            ),
            Self::NoticePaused => PromptSpec::new(
                "harness/run/notice-paused.md",
                Audience::Human,
                &["agent", "task", "until", "reason", "evidence"],
            ),
            Self::NoticeFailed => PromptSpec::new(
                "harness/run/notice-failed.md",
                Audience::Human,
                &["agent", "task", "message"],
            ),
            Self::NoticeNoStatus => PromptSpec::new(
                "harness/run/notice-no-status.md",
                Audience::Human,
                &["agent", "task", "exit"],
            ),
            Self::NoticeNoStatusSignal => PromptSpec::new(
                "harness/run/notice-no-status-signal.md",
                Audience::Human,
                &["agent", "task"],
            ),
            Self::NoticeStartFailed => PromptSpec::new(
                "harness/run/notice-start-failed.md",
                Audience::Human,
                &["agent", "task", "reason"],
            ),
            Self::NoticeRunFailed => PromptSpec::new(
                "harness/run/notice-run-failed.md",
                Audience::Human,
                &["agent", "task", "reason"],
            ),
            Self::NoticeLowDisk => PromptSpec::new(
                "harness/run/notice-low-disk.md",
                Audience::Human,
                &["free", "limit", "worktrees"],
            ),
            Self::NoticeDiskWait => {
                PromptSpec::new("harness/run/notice-disk-wait.md", Audience::Human, &["free", "min"])
            }
            Self::Plan => PromptSpec::new("harness/run/plan.md", Audience::Model, &[]),
            Self::PlanMissing => PromptSpec::new("harness/run/plan-missing.md", Audience::Model, &[]),
            Self::PlanUnreadable => PromptSpec::new("harness/run/plan-unreadable.md", Audience::Model, &["error"]),
            Self::PlanUnfinished => PromptSpec::new("harness/run/plan-unfinished.md", Audience::Model, &["steps"]),
            Self::PlanLostStep => PromptSpec::new("harness/run/plan-lost-step.md", Audience::Model, &["steps"]),
            Self::NoticePlanQuestion => PromptSpec::new(
                "harness/run/notice-plan-question.md",
                Audience::Human,
                &["task", "agent", "count", "reason"],
            ),
        }
    }
}

/// The briefing of a member's work as its lead is told: the facts of [`HarnessPrompt::Briefing`] without the prompt.
const fn lead_briefing_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/run/lead-briefing.md",
        Audience::Model,
        &[
            "reason",
            "task",
            "base",
            "branch",
            "commits",
            "uncommitted",
            "snapshot",
            "plan",
            "left_out",
            "messages",
        ],
    )
}
