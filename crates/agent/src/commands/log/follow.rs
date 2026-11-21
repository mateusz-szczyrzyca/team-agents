//! The feed of `agent log --follow`: the newest lines of what a scope covers, then every row and every step recorded
//! after them, each once.
//!
//! The rows of the daemon are followed by their database ID. The steps of a run are followed by their number in the
//! run, which the writer allocates in order, so a step is never printed twice and none is skipped, whatever the order
//! of the times of rows and steps. A run is told once, before the first of its steps that is printed.

#[cfg(test)]
mod tests;

use agent_core::ids::RoundId;
use agent_core::story::StepView;
use agent_runtime::history::{LogFilter, Reader};

use super::LogError;

/// What a follow has printed of its scope.
#[derive(Debug)]
pub(crate) struct Feed {}

impl Feed {
    /// Opens the feed of `filter`: the text of its newest `limit` lines, rows and steps together.
    ///
    /// # Errors
    /// [`LogError::Read`] when the history cannot be read.
    pub(crate) fn open(
        _reader: &Reader,
        _filter: LogFilter,
        _steps: StepView,
        _limit: u32,
    ) -> Result<(Self, String), LogError> {
        Ok((Self {}, String::new()))
    }

    /// The text of what was recorded since the last page; `waits` tells what a round waits for after a run of it ended.
    ///
    /// # Errors
    /// [`LogError::Read`] when the history cannot be read.
    pub(crate) fn poll(
        &mut self,
        _reader: &Reader,
        _waits: &mut dyn FnMut(&RoundId) -> String,
    ) -> Result<String, LogError> {
        Ok(String::new())
    }
}
