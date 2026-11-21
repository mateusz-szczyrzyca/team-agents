//! The words that name a task in a notification: its description, its item and the first sentences of its brief.

use agent_core::ids::ItemId;
use agent_core::state::split::{PartKey, SplitPlan};
use agent_core::task_list::TaskList;
use agent_core::task_title::brief_excerpt;

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
    pub(crate) fn new(list: &TaskList, item: Option<String>) -> Self {
        Self {
            short_desc: list.display_short_desc(),
            item,
            excerpt: brief_excerpt(list.brief()),
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
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired in the commit that names the item of a notice")
)]
pub(crate) fn item_line(list: &TaskList, splits: &[SplitPlan], item: ItemId, part: Option<PartKey>) -> Option<String> {
    let found = list.item(item)?;
    let of_part = part.and_then(|key| {
        let plan = splits.iter().find(|plan| plan.parent() == item)?;
        let (id, definition) = plan.entries().find(|(id, _)| id.key() == key)?;
        Some(format!("{id} {}", definition.title))
    });
    Some(of_part.unwrap_or_else(|| format!("{} {}", found.id, found.title)))
}

#[cfg(test)]
mod tests;
