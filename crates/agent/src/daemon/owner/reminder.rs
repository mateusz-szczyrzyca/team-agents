//! The daemon's side of the reminder of Serena: the reminder the history holds for the run that ended.
//!
//! When a run of a session has ended and the engine recorded the reminder of that session, the daemon reads that reminder
//! from the history of the run's team and sends the text of the catalog to the run's recipient on the bus, once, at the
//! first look at the task after the end.

use agent_core::serena::{ReminderText, ReminderTextError, SessionTally};
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};

mod held;
#[cfg(test)]
mod tests;

pub(crate) use agent_runtime::history::ReadError;
pub(crate) use held::reminder_of;

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
