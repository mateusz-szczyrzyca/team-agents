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
    /// The run could not be started or carried out.
    StartFailed {
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
    pub(super) fn render(
        &self,
        texts: &HarnessPrompts,
        agent: &str,
        task: &TaskId,
    ) -> Result<Notification, NoticeError> {
        let _ = (texts, agent, task);
        Ok(Notification::new("?", "?")?)
    }
}

#[cfg(test)]
mod tests;
