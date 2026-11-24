//! The daemon's side of the reminder of Serena: the reminder the history holds for the run that ended.
//!
//! When a run of a session has ended and the engine recorded the reminder of that session, the daemon reads that reminder
//! from the history of the run's team and sends the text of the catalog to the run's recipient on the bus, once, at the
//! first look at the task after the end.

use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::serena::ReminderTextError;
use agent_runtime::harness_prompts::HarnessPrompts;

mod held;
#[cfg(test)]
mod tests;

pub(crate) use agent_runtime::history::ReadError;
pub(crate) use held::reminder_of;

/// The text of the reminder of Serena for a session that made `searches` searches of code without Serena.
///
/// # Errors
/// [`ReminderTextError::Variables`] when the count is no value of the text of the reminder, and
/// [`ReminderTextError::Refused`] when the loader of the texts cannot render the reminder, with the loader's reason.
pub(super) fn text(searches: u64, texts: &HarnessPrompts) -> Result<String, ReminderTextError> {
    let values = Variables::new()
        .with("count", searches.to_string())
        .map_err(|source| ReminderTextError::Variables { source })?;
    texts
        .render(HarnessPrompt::SerenaRemind, &values)
        .map_err(|source| ReminderTextError::Refused {
            source: Box::new(source),
        })
}
