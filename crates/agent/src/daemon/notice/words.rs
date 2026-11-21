//! The words that name a task in a notification: its description, its item and the first sentences of its brief.

use agent_core::ids::ItemId;
use agent_core::state::split::{PartKey, SplitPlan};
use agent_core::task_list::TaskList;

/// What a notification says of its task in words, besides the identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskWords {
    /// The short description of the task.
    short_desc: String,
    /// The item or part the notification concerns, as one line of its number and its full title.
    item: Option<String>,
    /// The first sentences of the task's brief; empty when the brief is.
    excerpt: String,
}

impl TaskWords {
    /// The words of the task whose list is `list`, for a notification that concerns `item`.
    pub(crate) fn new(_list: &TaskList, _item: Option<String>) -> Self {
        Self {
            short_desc: String::new(),
            item: None,
            excerpt: String::new(),
        }
    }

    /// The short description of the task.
    pub(super) fn short_desc(&self) -> &str {
        &self.short_desc
    }

    /// The line of the item or part the notification concerns.
    pub(super) fn item(&self) -> Option<&str> {
        self.item.as_deref()
    }

    /// The first sentences of the task's brief.
    pub(super) fn excerpt(&self) -> &str {
        &self.excerpt
    }
}

/// The line that names `item` of `list` in a notification — its number and its full title — or the part `part` of it
/// that `splits` define; `None` when the list has no such item.
pub(crate) fn item_line(
    _list: &TaskList,
    _splits: &[SplitPlan],
    _item: ItemId,
    _part: Option<PartKey>,
) -> Option<String> {
    None
}

#[cfg(test)]
mod tests;
