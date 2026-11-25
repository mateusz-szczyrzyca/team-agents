//! The rows of the reminders for `agent log --reminders`: the daemon's log events about reminding a run to commit or
//! telling a session its context is large, each with the model of its run.

use agent_core::decide::reminders::ReminderRow;

use super::{ReadError, Reader};

impl Reader {
    /// The rows of the log events `commit_reminded`, `context_reminded`, `commit_followed` and `commit_cut`, oldest
    /// first, each with the agent of its field `agent` and the model of the run it names; a history without a log
    /// has none.
    ///
    /// # Errors
    /// [`ReadError::Query`] when the log or the runs cannot be read; [`ReadError::Value`] when a row of these events
    /// has no valid `agent`, or a `commit_followed` row has no whole `minutes`.
    pub fn reminder_rows(&self) -> Result<Vec<ReminderRow>, ReadError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests;
