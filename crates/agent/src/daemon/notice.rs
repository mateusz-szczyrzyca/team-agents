//! The notifications a run's end causes: which, and their texts from the catalog.
//!
//! Every wording is a template under `prompts/harness/run/`; this module chooses one, fills it and splits it into the
//! title (its first line) and the body.

use std::num::NonZeroU32;
use std::path::PathBuf;

use agent_core::decide::outcome::PauseReason;
use agent_core::ids::TaskId;
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::state::Timestamp;
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::notify::{InvalidNotification, Notification};

/// What the human is told about a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Notice {
    /// The agent wrote questions for the human.
    Questions {
        /// The round.
        round: NonZeroU32,
        /// The questions file.
        file: PathBuf,
    },
    /// The agent ended with questions, but their file cannot be used.
    QuestionsMissing {
        /// The questions file the round should have.
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
    /// A notification that was made where its text is known, such as the one of a team that is finished.
    Prepared(Notification),
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
        agent: &str,
        task: &TaskId,
    ) -> Result<Notification, NoticeError> {
        let common = Variables::new().with("agent", agent)?.with("task", task.as_str())?;
        let (text, values) = match self {
            Self::Prepared(notification) => return Ok(notification.clone()),
            Self::Questions { round, file } => (
                HarnessPrompt::NoticeQuestions,
                common
                    .with("round", round.to_string())?
                    .with("file", file.display().to_string())?,
            ),
            Self::QuestionsMissing { file, reason } => (
                HarnessPrompt::NoticeQuestionsMissing,
                common
                    .with("file", file.display().to_string())?
                    .with("reason", reason)?,
            ),
            Self::Hung { minutes, strikes } => (
                HarnessPrompt::NoticeHung,
                common
                    .with("minutes", minutes.to_string())?
                    .with("strikes", strikes.to_string())?,
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
            ),
            Self::Failed { message } => (HarnessPrompt::NoticeFailed, common.with("message", message)?),
            Self::NoStatus { exit: Some(code) } => {
                (HarnessPrompt::NoticeNoStatus, common.with("exit", code.to_string())?)
            }
            Self::NoStatus { exit: None } => (HarnessPrompt::NoticeNoStatusSignal, common),
            Self::StartFailed { reason } => (HarnessPrompt::NoticeStartFailed, common.with("reason", reason)?),
            Self::RunFailed { reason } => (HarnessPrompt::NoticeRunFailed, common.with("reason", reason)?),
        };
        let rendered = texts.render(text, &values)?;
        // The first line is the title, what follows is the body.
        let (title, body) = rendered.split_once('\n').unwrap_or((rendered.as_str(), ""));
        Ok(Notification::new(title.trim_end(), body.trim())?)
    }
}

#[cfg(test)]
mod tests;
