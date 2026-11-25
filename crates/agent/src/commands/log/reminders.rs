//! `agent log --reminders`: how the reminders of the harness did, agent by agent, from the log of the history.

use std::io::Write;
use std::path::Path;

use agent_core::decide::reminders::ReminderLine;
use agent_core::state::Timestamp;

use super::LogError;

/// Prints the table of the reminders of the last `days` days before `now`, read from the history in `history`.
///
/// # Errors
/// [`LogError::Read`] when the history cannot be read, [`LogError::Output`] when `out` refuses the text.
pub(crate) fn print_reminders(
    _history: &Path,
    days: u32,
    _now: Timestamp,
    out: &mut impl Write,
) -> Result<(), LogError> {
    out.write_all(render(&[], days).as_bytes()).map_err(LogError::Output)
}

/// The text of the table `lines` of the last `days` days: a header and a line for each agent and model, or one
/// sentence when there is none.
pub(crate) fn render(_lines: &[ReminderLine], _days: u32) -> String {
    String::new()
}

#[cfg(test)]
mod tests;
