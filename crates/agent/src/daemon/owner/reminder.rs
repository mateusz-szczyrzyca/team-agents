//! The daemon's side of the reminder of Serena: the reminder the history holds for the run that ended.
//!
//! When a run of a session has ended and the engine recorded the reminder of that session, the daemon reads that reminder
//! from the history of the run's team and sends the text of the catalog to the run's recipient on the bus, once, at the
//! first look at the task after the end.

use agent_core::log::LogEvent;
use agent_core::serena::{ReminderText, ReminderTextError, SessionTally};
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::run::serena::DueReminder;

use super::error_chain;

mod backlog;
mod held;
#[cfg(test)]
mod tests;
mod told;

pub(crate) use agent_runtime::history::ReadError;
pub(crate) use backlog::{Backlog, Failed};
pub(crate) use held::reminder_of;
pub(crate) use told::{is_told, mark_told};

/// Why the text of the reminder of Serena cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum TextError {
    /// The count of searches is no value of the text of the reminder.
    #[error("the values of the reminder of Serena are refused")]
    Values {
        /// What the values refused.
        source: ReminderTextError,
    },
    /// The loader of the texts cannot render the text of the reminder with its values.
    #[error("the text of the reminder of Serena cannot be rendered")]
    Render {
        /// What the loader refused.
        source: RenderTextError,
    },
}

/// The text of the reminder of Serena for a session that made `searches` searches of code without Serena.
///
/// # Errors
/// [`TextError::Values`] when the count is no value of the text of the reminder, and [`TextError::Render`] when the loader
/// of the texts cannot render the reminder.
pub(super) fn text(searches: u64, texts: &HarnessPrompts) -> Result<String, TextError> {
    let reminder =
        ReminderText::of(&SessionTally::counted_from(searches)).map_err(|source| TextError::Values { source })?;
    texts
        .render(reminder.text(), reminder.values())
        .map_err(|source| TextError::Render { source })
}

/// Why the reminder of Serena cannot be put on the bus.
#[derive(Debug, thiserror::Error)]
pub(super) enum PutError {
    /// The text of the reminder cannot be made.
    #[error("the reminder of Serena cannot be rendered")]
    Text {
        /// What refused the text.
        source: TextError,
    },
    /// The task has no recipient on the bus.
    #[error("the reminder of Serena has no recipient: the run reads it not")]
    Recipient {
        /// Why the task has none.
        source: super::OwnerError,
    },
    /// The bus refused the message.
    #[error("the reminder of Serena cannot be put on the bus: the run reads it not")]
    Bus {
        /// What the bus said.
        source: super::OwnerError,
    },
}

impl PutError {
    /// Logs the failure of the reminder `due`, once, where it is handled: a bus that is down is [`LogEvent::BusDown`], the
    /// rest is a task that failed at its reminder.
    pub(super) fn log(&self, due: &DueReminder) {
        let error = error_chain(self);
        match self {
            Self::Bus { .. } => tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                task = %due.task,
                run = %due.run,
                error = %error,
                "the reminder of Serena was not put on the bus"
            ),
            Self::Text { .. } | Self::Recipient { .. } => tracing::warn!(
                event = LogEvent::DaemonTaskFailed.as_str(),
                task = %due.task,
                run = %due.run,
                cli = due.cli.as_str(),
                searches = due.searches,
                operation = "serena_reminder",
                error = %error,
                "the reminder of Serena was not made ready for the bus"
            ),
        }
    }
}
