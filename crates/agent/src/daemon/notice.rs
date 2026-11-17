//! The notifications a run's end causes: which, and their texts from the catalog.
//!
//! Every wording is a template under `prompts/harness/run/`; this module chooses one, fills it and splits it into the
//! title (its first line) and the body.

use std::num::NonZeroU32;
use std::path::PathBuf;
use time::UtcOffset;

use agent_core::decide::outcome::PauseReason;
use agent_core::ids::{TaskId, TeamId};
use agent_core::kinds::CliKind;
use agent_core::notice_status::NoticeStatus;
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::state::Timestamp;
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::notify::{InvalidNotification, Notification};

/// Every notice that can be sent to the human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum NoticeKind {
    /// Questions were asked (`run/notice-questions`).
    Questions,
    /// An agent wrote questions, but the file cannot be read (`run/notice-questions-missing`).
    QuestionsMissing,
    /// The task is hung after watchdog strikes (`run/notice-hung`).
    Hung,
    /// Turn limit continuations exhausted (`run/notice-turn-limit-exhausted`).
    TurnLimitExhausted,
    /// Turn limit run had no session (`run/notice-turn-limit-no-session`).
    TurnLimitNoSession,
    /// Task paused for provider limits (`run/notice-paused`).
    Paused,
    /// Task reported failed (`run/notice-failed`).
    Failed,
    /// Run ended without status code (`run/notice-no-status`).
    NoStatus,
    /// Run ended by signal without status (`run/notice-no-status-signal`).
    NoStatusSignal,
    /// Run could not be started (`run/notice-start-failed`).
    StartFailed,
    /// Run failed while running (`run/notice-run-failed`).
    RunFailed,
    /// Questions file incomplete (`questions/notice-incomplete`).
    Incomplete,
    /// Reviewers asked for changes, human decision needed (`review/notice-decision`).
    Decision,
    /// Lead task did not ask checks questions (`checks/notice-not-asked`).
    NotAsked,
    /// Pull request approved and ready to merge (`pr/notice-approved`).
    Approved,
    /// Pull request merge failed on GitHub (`pr/notice-merge-failed`).
    MergeFailed,
    /// Pull request was closed without merge (`pr/notice-closed`).
    Closed,
    /// Pull request branch could not be published (`pr/notice-publish-failed`).
    PublishFailed,
    /// Reviewer ended without a verdict (`review/notice-no-verdict`).
    NoVerdict,
    /// Pull request merged (`pr/notice-merged`).
    Merged,
    /// Team asleep (`pr/notice-team-asleep`).
    TeamAsleep,
    /// Team done (`team/notice-team-done`).
    TeamDone,
}

impl NoticeKind {
    /// The status of this notice.
    #[must_use]
    pub(super) const fn status(self) -> NoticeStatus {
        match self {
            Self::Questions | Self::Incomplete => NoticeStatus::Questions,
            Self::Decision | Self::NotAsked => NoticeStatus::Decision,
            Self::Failed
            | Self::RunFailed
            | Self::StartFailed
            | Self::NoStatus
            | Self::NoStatusSignal
            | Self::QuestionsMissing
            | Self::TurnLimitExhausted
            | Self::TurnLimitNoSession => NoticeStatus::Failed,
            Self::Hung => NoticeStatus::Hung,
            Self::Paused => NoticeStatus::Paused,
            Self::Approved => NoticeStatus::Approved,
            Self::MergeFailed => NoticeStatus::NotMerged,
            Self::Closed => NoticeStatus::Closed,
            Self::PublishFailed => NoticeStatus::NotPublished,
            Self::NoVerdict => NoticeStatus::NoVerdict,
            Self::Merged => NoticeStatus::Merged,
            Self::TeamAsleep => NoticeStatus::Asleep,
            Self::TeamDone => NoticeStatus::Done,
        }
    }
}

/// The facts about the task and environment needed to lay out a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NoticeContext<'a> {
    /// The repository name.
    pub(super) repo: &'a str,
    /// The agent name.
    pub(super) agent: &'a str,
    /// The CLI kind of the agent, if known.
    pub(super) cli: Option<CliKind>,
    /// The model name of the agent, if configured.
    pub(super) model: Option<&'a str>,
    /// The time the event occurred.
    pub(super) time: Timestamp,
    /// The host's local offset from UTC.
    pub(super) offset: UtcOffset,
    /// The task ID.
    pub(super) task: &'a TaskId,
    /// The team ID, if the task belongs to a team.
    pub(super) team: Option<&'a TeamId>,
    /// The address of the pull request, if applicable.
    pub(super) pr: Option<&'a str>,
}

/// Lays out a notification for the human.
///
/// # Errors
/// [`InvalidNotification`] when title or body violates limits or format rules.
pub(super) fn layout_notification(
    notice: NoticeKind,
    context: &NoticeContext<'_>,
    rendered: &str,
) -> Result<Notification, InvalidNotification> {
    let (note, original_body) = rendered.split_once('\n').unwrap_or((rendered, ""));
    let note = note.trim_end();
    let original_body = original_body.trim();

    let status = notice.status();
    let title = format!("{} {}", status.icon(), context.repo);

    let local_dt = context.time.as_utc().to_offset(context.offset);
    let time_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local_dt.year(),
        u8::from(local_dt.month()),
        local_dt.day(),
        local_dt.hour(),
        local_dt.minute(),
    );

    let agent_str = match (context.cli, context.model) {
        (Some(cli), Some(model)) => format!("{} ({cli}, {model})", context.agent),
        (Some(cli), None) => format!("{} ({cli})", context.agent),
        (None, Some(model)) => format!("{} ({model})", context.agent),
        (None, None) => context.agent.to_string(),
    };

    let mut body = format!(
        "Repo: {}\nAgent: {agent_str}\nTime: {time_str}\nTask: {}",
        context.repo, context.task
    );
    if let Some(team) = context.team {
        body.push_str("\nTeam: ");
        body.push_str(team.as_str());
    }
    body.push_str("\nStatus: ");
    body.push_str(status.icon());
    body.push(' ');
    body.push_str(status.label());
    body.push_str("\nNote: ");
    body.push_str(note);
    if let Some(pr) = context.pr {
        body.push_str("\nPR: ");
        body.push_str(pr);
    }

    if !original_body.is_empty() {
        body.push_str("\n\n");
        body.push_str(original_body);
    }

    Notification::new(&title, &body)
}

/// What the human is told about a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Notice {
    /// The agent wrote questions for the human.
    Questions {
        /// The number of the questions.
        number: NonZeroU32,
        /// The questions file.
        file: PathBuf,
    },
    /// The agent ended with questions, but their file cannot be used.
    QuestionsMissing {
        /// The questions file the questions should have.
        file: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The task was marked failed after the watchdog's last strike.
    Hung {
        /// The watchdog's limit, in minutes.
        minutes: u64,
        /// The strikes that were allowed.
        strikes: u32,
    },
    /// Consecutive turn-limit endings exhausted the automatic resumes.
    TurnLimitExhausted {
        /// Number of consecutive endings, including the refused continuation.
        count: u64,
        /// Configured maximum automatic continuations.
        limit: u32,
    },
    /// A turn-limited run had no session ID to continue.
    TurnLimitNoSession,
    /// The task waits for a provider's limit to end.
    Paused {
        /// When the task goes on.
        until: Timestamp,
        /// Which limit.
        reason: PauseReason,
        /// What the provider said.
        evidence: String,
    },
    /// The agent reported `STATUS: FAILED`.
    Failed {
        /// The end of its final message.
        message: String,
    },
    /// The run ended without a status line.
    NoStatus {
        /// The exit code; `None` when a signal ended the process.
        exit: Option<i32>,
    },
    /// The run could not be started.
    StartFailed {
        /// Why, as the chain of its errors.
        reason: String,
    },
    /// The run started but could not be carried out to its end.
    RunFailed {
        /// Why, as the chain of its errors.
        reason: String,
    },
}

/// Why a notice cannot be made into a notification.
#[derive(Debug, thiserror::Error)]
pub(super) enum NoticeError {
    /// A variable name of the program is not valid.
    #[error("a variable of a notification text is not valid")]
    Variable(#[from] VariablesError),
    /// The text cannot be rendered.
    #[error("cannot render a notification text")]
    Render(#[from] RenderTextError),
    /// The rendered text is not a notification.
    #[error("the notification text is not valid")]
    Invalid(#[from] InvalidNotification),
}

impl Notice {
    /// The notification of this notice about `task`, whose agent is `agent`.
    ///
    /// # Errors
    /// [`NoticeError`] when the text of the notice cannot be rendered or is not a notification.
    pub(super) fn render(
        &self,
        texts: &HarnessPrompts,
        context: &NoticeContext<'_>,
    ) -> Result<Notification, NoticeError> {
        let common = Variables::new()
            .with("agent", context.agent)?
            .with("task", context.task.as_str())?;
        let (text, values, kind) = match self {
            Self::Questions { number, file } => (
                HarnessPrompt::NoticeQuestions,
                common
                    .with("number", number.to_string())?
                    .with("file", file.display().to_string())?,
                NoticeKind::Questions,
            ),
            Self::QuestionsMissing { file, reason } => (
                HarnessPrompt::NoticeQuestionsMissing,
                common
                    .with("file", file.display().to_string())?
                    .with("reason", reason)?,
                NoticeKind::QuestionsMissing,
            ),
            Self::Hung { minutes, strikes } => (
                HarnessPrompt::NoticeHung,
                common
                    .with("minutes", minutes.to_string())?
                    .with("strikes", strikes.to_string())?,
                NoticeKind::Hung,
            ),
            Self::TurnLimitExhausted { count, limit } => (
                HarnessPrompt::NoticeTurnLimitExhausted,
                common
                    .with("count", count.to_string())?
                    .with("limit", limit.to_string())?,
                NoticeKind::TurnLimitExhausted,
            ),
            Self::TurnLimitNoSession => (
                HarnessPrompt::NoticeTurnLimitNoSession,
                common,
                NoticeKind::TurnLimitNoSession,
            ),
            Self::Paused {
                until,
                reason,
                evidence,
            } => (
                HarnessPrompt::NoticePaused,
                common
                    .with("until", until.to_string())?
                    .with("reason", reason.to_string())?
                    .with("evidence", evidence)?,
                NoticeKind::Paused,
            ),
            Self::Failed { message } => (
                HarnessPrompt::NoticeFailed,
                common.with("message", message)?,
                NoticeKind::Failed,
            ),
            Self::NoStatus { exit: Some(code) } => (
                HarnessPrompt::NoticeNoStatus,
                common.with("exit", code.to_string())?,
                NoticeKind::NoStatus,
            ),
            Self::NoStatus { exit: None } => (HarnessPrompt::NoticeNoStatusSignal, common, NoticeKind::NoStatusSignal),
            Self::StartFailed { reason } => (
                HarnessPrompt::NoticeStartFailed,
                common.with("reason", reason)?,
                NoticeKind::StartFailed,
            ),
            Self::RunFailed { reason } => (
                HarnessPrompt::NoticeRunFailed,
                common.with("reason", reason)?,
                NoticeKind::RunFailed,
            ),
        };
        let rendered = texts.render(text, &values)?;
        Ok(layout_notification(kind, context, &rendered)?)
    }
}

#[cfg(test)]
mod tests;
