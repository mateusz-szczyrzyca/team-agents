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
    /// How a run that has Serena finds code with it: a symbol through its language server, and `grep` only for text.
    Serena,
    /// The reminder of a run with Serena that searches code without it, with the count of those searches.
    SerenaRemind,
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
    /// The note a run that was retried after it reported `STATUS: FAILED` without calling a tool is told.
    NoToolCall,
    /// The briefing of a task started in a new session after its runs went bad.
    Briefing,
    /// What the runs of a member's task left, as its lead is told when the work is its own: the briefing without the
    /// task's prompt.
    LeadBriefing,
    /// The section a lead's briefing of a member's work ends with: what the member's last run said at its end, so that the
    /// lead reads the best account of where the work stands.
    LeadBriefingLastMessage,
    /// The paragraph a briefing opens with when its session outgrew its agent's bound: why the session is new, with the
    /// context in tokens that ended the old one.
    BriefingContext,
    /// The paragraph a briefing opens with when its session was replaced on a size nobody could read: it may have outgrown its
    /// bound.
    BriefingContextUnknown,
    /// The section a briefing of a new session ends with: the agent's last final message of the session before, whole (read
    /// from the run's outcome file).
    BriefingLastMessage,
    /// The same section for a message read from the history, which may be cut: its heading does not say whole.
    BriefingLastMessageHistory,
    /// The note after the last message of the session before when the history cut it at its limit: the run wrote no outcome
    /// file that holds it whole.
    BriefingMessageCut,
    /// The note of a briefing whose list of unread messages leaves older ones out.
    BriefingMessagesLeftOut,
    /// The note that older runs of the task are left out of the briefing's list of runs.
    BriefingRunsLeftOut,
    /// The note that older answers of the human are left out of the briefing's list of answers.
    BriefingAnswersLeftOut,
    /// The heading of the briefing's steps whose check passed.
    BriefingPlanConfirmed,
    /// The heading of the briefing's steps the agent declared done with no check that passed or failed.
    BriefingPlanDeclared,
    /// The heading of the briefing's steps the agent declared done and whose check failed.
    BriefingPlanMismatch,
    /// The heading of the briefing's steps that are not done.
    BriefingPlanNotDone,
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
    /// The two parts every notification about a task has: its title line is the line that gives the task's ID, its body
    /// the block that ends the notification with the first sentences of the task's brief.
    NoticeTask,
    /// The notification that a task was marked failed after the watchdog's last strike.
    NoticeHung,
    /// The notification that a task is paused for a provider's limit.
    NoticePaused,
    /// The notification that an agent reported `STATUS: FAILED`.
    NoticeFailed,
    /// The notification that an agent reported `STATUS: FAILED` twice without calling a tool, after its one retry.
    NoticeFailedNoTool,
    /// The notification that a run ended without a status line, with an exit code.
    NoticeNoStatus,
    /// The notification that a run a signal ended had no status line.
    NoticeNoStatusSignal,
    /// The notification that a run could not be started.
    NoticeStartFailed,
    /// The notification that a run that started could not be carried out to its end.
    NoticeRunFailed,
    /// The notification that a task was handed to another agent after its original agent's provider refused it.
    NoticeAgentChanged,
    /// The notification that no agent is free to take over a task after its agent's provider refused it.
    NoticeNoFreeAgent,
    /// The notification that free disk space is below `low_free_gb`.
    NoticeLowDisk,
    /// The removal of a team, its backup and released bytes.
    NoticeTeamRemoved,
    /// An idle team could not safely be removed.
    NoticeIdleRefused,
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
    /// What a run sent back because a step check failed is told.
    PlanMismatch,
    /// What a run whose step check was refused because it ran the gate is told on its next run.
    PlanGateRefused,
    /// The questions file the harness writes for the human when a task was sent back for its plan as often as
    /// `[plan] bounces` allows.
    NoticePlanQuestion,
    /// The question to the human what becomes of a task after the last step of the ladder went bad too.
    NoticeLastStep,
    /// What the hook of `git commit` tells an agent whose commit it refused: the findings, and how to make the commit
    /// the rule allows. Its only variable holds the lines of [`HarnessPrompt::CommitRefusedLines`],
    /// [`HarnessPrompt::CommitRefusedWhole`] and [`HarnessPrompt::CommitRefusedMixed`].
    CommitRefused,
    /// The finding of a refused commit that takes some lines out of a frozen test.
    CommitRefusedLines,
    /// The finding of a refused commit that takes a whole frozen test out: deletes it, or replaces a binary one.
    CommitRefusedWhole,
    /// The finding of a refused `TEST-FIX:` commit that also changes files that are not tests.
    CommitRefusedMixed,
}

impl HarnessPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::Context,
        Self::Serena,
        Self::SerenaRemind,
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
        Self::NoToolCall,
        Self::Briefing,
        Self::LeadBriefing,
        Self::LeadBriefingLastMessage,
        Self::BriefingContext,
        Self::BriefingContextUnknown,
        Self::BriefingLastMessage,
        Self::BriefingLastMessageHistory,
        Self::BriefingMessageCut,
        Self::BriefingMessagesLeftOut,
        Self::BriefingRunsLeftOut,
        Self::BriefingAnswersLeftOut,
        Self::BriefingPlanConfirmed,
        Self::BriefingPlanDeclared,
        Self::BriefingPlanMismatch,
        Self::BriefingPlanNotDone,
        Self::NoticeTurnLimitExhausted,
        Self::NoticeTurnLimitNoSession,
        Self::PromptFile,
        Self::NoticeQuestions,
        Self::NoticeQuestionsMissing,
        Self::NoticeTask,
        Self::NoticeHung,
        Self::NoticePaused,
        Self::NoticeFailed,
        Self::NoticeFailedNoTool,
        Self::NoticeNoStatus,
        Self::NoticeNoStatusSignal,
        Self::NoticeStartFailed,
        Self::NoticeRunFailed,
        Self::NoticeAgentChanged,
        Self::NoticeNoFreeAgent,
        Self::NoticeLowDisk,
        Self::NoticeTeamRemoved,
        Self::NoticeIdleRefused,
        Self::NoticeDiskWait,
        Self::Plan,
        Self::PlanMissing,
        Self::PlanUnreadable,
        Self::PlanUnfinished,
        Self::PlanLostStep,
        Self::PlanMismatch,
        Self::PlanGateRefused,
        Self::NoticePlanQuestion,
        Self::NoticeLastStep,
        Self::CommitRefused,
        Self::CommitRefusedLines,
        Self::CommitRefusedWhole,
        Self::CommitRefusedMixed,
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
            Self::Serena => PromptSpec::new("harness/run/serena.md", Audience::Model, &[]),
            Self::SerenaRemind => PromptSpec::new("harness/run/serena-remind.md", Audience::Model, &["count"]),
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
            Self::NoToolCall => PromptSpec::new("harness/run/no-tool-call.md", Audience::Model, &["agent"]),
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
                    "runs",
                    "runs_left_out",
                    "answers",
                    "answers_left_out",
                    "left_out",
                    "messages",
                ],
            ),
            Self::LeadBriefing => lead_briefing_spec(),
            Self::LeadBriefingLastMessage => PromptSpec::new(
                "harness/run/lead-briefing-last-message.md",
                Audience::Model,
                &["message"],
            ),
            Self::BriefingMessagesLeftOut => {
                PromptSpec::new("harness/run/briefing-messages-left-out.md", Audience::Model, &[])
            }
            Self::BriefingContext => PromptSpec::new("harness/run/briefing-context.md", Audience::Model, &["context"]),
            Self::BriefingContextUnknown => {
                PromptSpec::new("harness/run/briefing-context-unknown.md", Audience::Model, &[])
            }
            Self::BriefingLastMessage => {
                PromptSpec::new("harness/run/briefing-last-message.md", Audience::Model, &["message"])
            }
            Self::BriefingLastMessageHistory => PromptSpec::new(
                "harness/run/briefing-last-message-history.md",
                Audience::Model,
                &["message"],
            ),
            Self::BriefingMessageCut => {
                PromptSpec::new("harness/run/briefing-message-cut.md", Audience::Model, &["limit"])
            }
            Self::BriefingRunsLeftOut => PromptSpec::new("harness/run/briefing-runs-left-out.md", Audience::Model, &[]),
            Self::BriefingAnswersLeftOut => {
                PromptSpec::new("harness/run/briefing-answers-left-out.md", Audience::Model, &[])
            }
            Self::BriefingPlanConfirmed => {
                PromptSpec::new("harness/run/briefing-plan-confirmed.md", Audience::Model, &[])
            }
            Self::BriefingPlanDeclared => {
                PromptSpec::new("harness/run/briefing-plan-declared.md", Audience::Model, &[])
            }
            Self::BriefingPlanMismatch => {
                PromptSpec::new("harness/run/briefing-plan-mismatch.md", Audience::Model, &[])
            }
            Self::BriefingPlanNotDone => PromptSpec::new("harness/run/briefing-plan-not-done.md", Audience::Model, &[]),
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
            Self::NoticeTask => PromptSpec::new("harness/run/notice-task.md", Audience::Human, &["task", "excerpt"]),
            Self::NoticeHung => PromptSpec::new(
                "harness/run/notice-hung.md",
                Audience::Human,
                &["agent", "task", "minutes", "strikes"],
            ),
            Self::NoticePaused => PromptSpec::new(
                "harness/run/notice-paused.md",
                Audience::Human,
                &["agent", "task", "until", "reason", "kind", "evidence"],
            ),
            Self::NoticeFailed => PromptSpec::new(
                "harness/run/notice-failed.md",
                Audience::Human,
                &["agent", "task", "message"],
            ),
            Self::NoticeFailedNoTool => PromptSpec::new(
                "harness/run/notice-failed-no-tool.md",
                Audience::Human,
                &["agent", "task", "cli", "model", "message"],
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
            Self::NoticeAgentChanged => PromptSpec::new(
                "harness/run/notice-agent-changed.md",
                Audience::Human,
                &[
                    "agent",
                    "task",
                    "refused_agent",
                    "refused_provider",
                    "return_at",
                    "new_agent",
                ],
            ),
            Self::NoticeNoFreeAgent => PromptSpec::new(
                "harness/run/notice-no-free-agent.md",
                Audience::Human,
                &["agent", "task", "refused_agent", "refused_provider", "return_at"],
            ),
            Self::NoticeIdleRefused => PromptSpec::new(
                "harness/run/notice-idle-refused.md",
                Audience::Human,
                &["team", "reason"],
            ),
            Self::NoticeTeamRemoved => PromptSpec::new(
                "harness/run/notice-removed.md",
                Audience::Human,
                &["team", "reason", "bundle", "bytes"],
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
            Self::PlanMismatch => PromptSpec::new(
                "harness/run/plan-mismatch.md",
                Audience::Model,
                &["check", "output", "reason", "step", "title"],
            ),
            Self::PlanGateRefused => PromptSpec::new(
                "harness/run/plan-gate-refused.md",
                Audience::Model,
                &["command", "step"],
            ),
            Self::NoticePlanQuestion => PromptSpec::new(
                "harness/run/notice-plan-question.md",
                Audience::Human,
                &["task", "agent", "count", "reason"],
            ),
            Self::NoticeLastStep => last_step_spec(),
            Self::CommitRefused => PromptSpec::new("harness/run/commit-refused.md", Audience::Model, &["findings"]),
            Self::CommitRefusedLines => PromptSpec::new(
                "harness/run/commit-refused-lines.md",
                Audience::Model,
                &["path", "red", "count"],
            ),
            Self::CommitRefusedWhole => {
                PromptSpec::new("harness/run/commit-refused-whole.md", Audience::Model, &["path", "red"])
            }
            Self::CommitRefusedMixed => {
                PromptSpec::new("harness/run/commit-refused-mixed.md", Audience::Model, &["files"])
            }
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
            "runs",
            "runs_left_out",
            "answers",
            "answers_left_out",
            "left_out",
            "messages",
        ],
    )
}

/// The question to the human after the last step of the ladder: the task, the agent that holds it, those that held it
/// before, and why its runs went bad.
const fn last_step_spec() -> PromptSpec {
    PromptSpec::new(
        "harness/run/notice-last-step.md",
        Audience::Human,
        &["task", "agent", "held", "reason"],
    )
}
