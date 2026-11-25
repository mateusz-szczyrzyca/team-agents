//! The notifications a run's end causes: which, and their texts from the catalog.
//!
//! Every wording is a template under `prompts/harness/run/`; this module chooses one, fills it and splits it into the
//! title (its first line) and the body.

mod words;

#[cfg(test)]
pub(crate) mod sample;

use std::num::NonZeroU32;
use std::path::PathBuf;

use agent_core::decide::outcome::PauseReason;
use agent_core::decide::refusal;
use agent_core::ids::{TaskId, TeamId};
use agent_core::kinds::CliKind;
use agent_core::local_time::Zone;
use agent_core::notice_status::NoticeStatus;
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::state::Timestamp;
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::notify::{InvalidNotification, Notification};

pub(crate) use words::{TaskWords, TaskWordsError, item_line, item_of_notice};

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
    /// Task reported failed twice without a tool (`run/notice-failed-no-tool`).
    FailedNoTool,
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
    /// An unanswered second opinion needs human attention (`review/notice-opinion`).
    Opinion,
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
            Self::Decision | Self::NotAsked | Self::Opinion => NoticeStatus::Decision,
            Self::Failed
            | Self::FailedNoTool
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
///
/// The type parameter `Z` defaults to [`Zone`] for daemon callers while accepting [`time::UtcOffset`] via
/// `From<UtcOffset> for Zone` so that the existing tests in `words/catalog_tests.rs` and `words/legacy_tests.rs` keep
/// passing `offset: UtcOffset` without modification.
#[derive(Debug, Clone)]
pub(super) struct NoticeContext<'a, Z = Zone> {
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
    /// The zone of the host: the times a notification shows are written in it, with the offset of each moment.
    /// Named `offset` so that existing tests in `words/catalog_tests.rs` and `words/legacy_tests.rs` compile unchanged.
    pub(super) offset: Z,
    /// The task ID.
    pub(super) task: &'a TaskId,
    /// The team ID, if the task belongs to a team.
    pub(super) team: Option<&'a TeamId>,
    /// The address of the pull request, if applicable.
    pub(super) pr: Option<&'a str>,
    /// What the notification says of its task in words.
    pub(super) words: TaskWords,
}

/// Formats the optional CLI and model shared by notification and review headers.
pub(super) fn client_label(cli: Option<CliKind>, model: Option<&str>) -> String {
    match (cli, model) {
        (Some(cli), Some(model)) => format!("({cli}, {model})"),
        (Some(cli), None) => format!("({cli})"),
        (None, Some(model)) => format!("({model})"),
        (None, None) => String::new(),
    }
}

/// Lays out a notification for the human.
///
/// # Errors
/// [`InvalidNotification`] when title or body violates limits or format rules.
pub(super) fn layout_notification<Z: Into<Zone> + Copy>(
    notice: NoticeKind,
    context: &NoticeContext<'_, Z>,
    rendered: &str,
) -> Result<Notification, InvalidNotification> {
    let (note, original_body) = rendered.split_once('\n').unwrap_or((rendered, ""));
    let note = note.trim_end();
    let original_body = original_body.trim();

    let status = notice.status();
    let title = format!("{} {}", status.icon(), context.repo);

    let time_str = context.offset.into().stamp_minute(context.time);

    let client = client_label(context.cli, context.model);
    let agent_str = if client.is_empty() {
        context.agent.to_owned()
    } else {
        format!("{} {client}", context.agent)
    };

    let mut head = format!(
        "Repo: {}\nTime: {time_str}\nTask: {}\nAgent: {agent_str}\n{}",
        context.repo,
        context.words.short_desc(),
        context.words.id_line()
    );
    if let Some(team) = context.team {
        head.push_str("\nTeam: ");
        head.push_str(team.as_str());
    }
    head.push_str("\nStatus: ");
    head.push_str(status.icon());
    head.push(' ');
    head.push_str(status.label());
    head.push_str("\nNote: ");
    head.push_str(note);
    if let Some(pr) = context.pr {
        head.push_str("\nPR: ");
        head.push_str(pr);
    }

    let sections = [
        Some(head.as_str()),
        context.words.item(),
        Some(original_body),
        context.words.tail(),
    ];
    let body = sections
        .into_iter()
        .flatten()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

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
        /// The kind read from the provider fields.
        kind: refusal::Kind,
        /// What the provider said.
        evidence: String,
    },
    /// The agent reported `STATUS: FAILED`.
    Failed {
        /// The end of its final message.
        message: String,
    },
    /// The agent reported `STATUS: FAILED` twice without calling a tool step, after its one retry of the first.
    FailedNoTool {
        /// The end of the last run's final message, what the agent said instead of calling a tool.
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
                kind,
                evidence,
            } => (
                HarnessPrompt::NoticePaused,
                common
                    .with("until", context.offset.stamp_minute(*until))?
                    .with("reason", reason.to_string())?
                    .with("kind", kind.words())?
                    .with(
                        "evidence",
                        evidence
                            .strip_prefix(&format!("{}: ", kind.words()))
                            .unwrap_or(evidence),
                    )?,
                NoticeKind::Paused,
            ),
            Self::Failed { message } => (
                HarnessPrompt::NoticeFailed,
                common.with("message", message)?,
                NoticeKind::Failed,
            ),
            Self::FailedNoTool { message } => {
                let cli = context
                    .cli
                    .map_or_else(|| "configured".to_owned(), |cli| cli.to_string());
                let model = context.model.unwrap_or("configured");
                (
                    HarnessPrompt::NoticeFailedNoTool,
                    common
                        .with("message", message)?
                        .with("cli", cli)?
                        .with("model", model)?,
                    NoticeKind::FailedNoTool,
                )
            }
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
