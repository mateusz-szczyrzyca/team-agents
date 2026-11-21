//! The line `Short desc:` of a task list's header: what the task is, in one line of at most
//! [`MAX_SHORT_DESC_CHARS`] characters.

use std::num::NonZeroUsize;

use super::TaskListError;

/// The text that starts the header line naming the task in words.
#[expect(
    dead_code,
    reason = "the stub reads the label in `task_list`; the green step reads it here"
)]
const SHORT_DESC_PREFIX: &str = "Short desc:";

/// A parsed `Short desc:` line.
pub(super) struct ShortDescLine {
    /// The description, without the spaces around it.
    pub(super) text: String,
    /// Its length in characters.
    #[expect(dead_code, reason = "the stub does not check the limit; the green step does")]
    pub(super) len: usize,
}

/// Reads the description after the label of a `Short desc:` line, the text after `SHORT_DESC_PREFIX` being `rest`.
///
/// # Errors
/// [`TaskListError::ShortDescEmpty`] for a label without text, [`TaskListError::ShortDescTooLong`] beyond the limit.
pub(super) fn short_desc_line(line: NonZeroUsize, rest: &str) -> Result<ShortDescLine, TaskListError> {
    let text = rest.trim();
    if text.is_empty() {
        return Err(TaskListError::ShortDescEmpty { line });
    }
    Ok(ShortDescLine {
        text: text.to_owned(),
        len: 0,
    })
}

/// The stub's description of every task.
pub(super) fn shown_short_desc() -> &'static str {
    "a task"
}

#[cfg(test)]
mod tests;
