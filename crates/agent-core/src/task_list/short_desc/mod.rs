//! The line `Short desc:` of a task list's header.

use std::num::NonZeroUsize;

use super::{TaskList, TaskListError};

/// The text that starts the header line naming the task in words.
const SHORT_DESC_PREFIX: &str = "Short desc:";

/// A parsed `Short desc:` line.
pub(super) struct ShortDescLine {
    /// The description, without the spaces around it.
    pub(super) text: String,
    /// Its length in characters.
    #[expect(
        dead_code,
        reason = "the length is wrong in the stub; the green step checks the limit with it"
    )]
    pub(super) len: usize,
}

/// Reads the description after the label of a `Short desc:` line, the text after `SHORT_DESC_PREFIX` being `rest`.
pub(super) fn short_desc_line(line: NonZeroUsize, rest: &str) -> Result<ShortDescLine, TaskListError> {
    // Stub: accepts any text but blanks, with the text's words only.
    if rest.trim().is_empty() {
        return Err(TaskListError::ShortDescEmpty { line });
    }
    Ok(ShortDescLine {
        text: rest.trim().to_owned(),
        len: 0,
    })
}

#[cfg(test)]
mod tests;
