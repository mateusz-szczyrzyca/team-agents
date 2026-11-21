//! The words that name a task in a notification: its description, its item and the first sentences of its brief.

use std::collections::BTreeSet;

use agent_core::ids::{ItemId, TaskId};
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::state::split::{PartKey, SplitPlan};
use agent_core::state::{Completion, ItemPhase, ItemProgress, Team};
use agent_core::task_list::TaskList;
use agent_core::task_title::brief_excerpt;
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};

/// Why the words of a task cannot be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TaskWordsError {
    /// A variable of a text is not valid.
    #[error("a variable of a text of the task's words is not valid")]
    Variable(#[from] VariablesError),
    /// A text of the catalog cannot be rendered.
    #[error("cannot render a text of the task's words")]
    Render(#[from] RenderTextError),
}

/// What a notification says of its task in words, besides the lines that identify it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskWords {
    /// The short description of the task.
    short_desc: String,
    /// The item or part the notification concerns, as one line of its number and its full title.
    item: Option<String>,
    /// The first sentences of the task's brief; empty when the brief has no words.
    excerpt: String,
    /// The line of the catalog that gives the task's ID.
    id_line: String,
    /// The end of the notification, from the catalog: the first sentences of the task's brief; none when the brief
    /// has no words.
    tail: Option<String>,
}

impl TaskWords {
    /// The words of the task `task` whose list is `list`, for a notification that concerns `item`, with the lines that
    /// the catalog gives to the task's ID and to the end of the notification.
    ///
    /// # Errors
    /// [`TaskWordsError`] when a text of the catalog cannot be rendered.
    pub(crate) fn rendered(
        list: &TaskList,
        item: Option<String>,
        task: &TaskId,
        texts: &HarnessPrompts,
    ) -> Result<Self, TaskWordsError> {
        let id = Variables::new().with("task", task.as_str())?;
        let id_line = texts.render(HarnessPrompt::NoticeTaskId, &id)?.trim_end().to_owned();
        let excerpt = brief_excerpt(list.brief());
        let tail = if excerpt.is_empty() {
            None
        } else {
            let values = id.with("excerpt", excerpt.as_str())?;
            Some(
                texts
                    .render(HarnessPrompt::NoticeTaskTail, &values)?
                    .trim_end()
                    .to_owned(),
            )
        };
        Ok(Self {
            short_desc: list.display_short_desc(),
            item,
            excerpt,
            id_line,
            tail,
        })
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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the tests of the words read the excerpt apart from its text"
        )
    )]
    pub(super) fn excerpt(&self) -> &str {
        &self.excerpt
    }

    /// The line that gives the task's ID.
    pub(super) fn id_line(&self) -> &str {
        &self.id_line
    }

    /// The end of the notification, if the task's brief has words.
    pub(super) fn tail(&self) -> Option<&str> {
        self.tail.as_deref()
    }
}

/// The line that names `item` of `list` in a notification — its number and its full title — or the part `part` of it
/// that `splits` define; `None` when the list has no such item.
pub(crate) fn item_line(list: &TaskList, splits: &[SplitPlan], item: ItemId, part: Option<PartKey>) -> Option<String> {
    let found = list.item(item)?;
    let of_part = part.and_then(|key| {
        let plan = splits.iter().find(|plan| plan.parent() == item)?;
        let (id, definition) = plan.entries().find(|(id, _)| id.key() == key)?;
        Some(format!("{id} {}", definition.title))
    });
    Some(of_part.unwrap_or_else(|| format!("{} {}", found.id, found.title)))
}

/// The item a notification is about, from what the notification says itself: its `item` or `items` (when exactly one
/// item is named), its `round` or `rounds`, or its pull request `pr`. A notification about several items, or about none,
/// names no item.
pub(crate) fn item_of_notice(team: &Team, values: &Variables) -> Option<ItemId> {
    let value = |name: &str| {
        values
            .names()
            .find(|candidate| candidate.as_str() == name)
            .and_then(|candidate| values.get(candidate))
    };
    let items = if let Some(text) = value("item").or_else(|| value("items")) {
        listed(text, |entry| entry.parse().ok())?
    } else if let Some(text) = value("round").or_else(|| value("rounds")) {
        listed(text, |entry| {
            let id = entry.parse().ok()?;
            team.round(&id)?.item()
        })?
    } else {
        let number = value("pr")?;
        team.items()
            .iter()
            .filter(|progress| pull_request_number(progress.phase()).is_some_and(|found| found == number))
            .map(ItemProgress::id)
            .collect()
    };
    let mut items = items.into_iter();
    match (items.next(), items.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// The distinct items that the comma-separated `text` names through `item_of`; none when an entry names none.
fn listed(text: &str, item_of: impl Fn(&str) -> Option<ItemId>) -> Option<BTreeSet<ItemId>> {
    text.split(',').map(|entry| item_of(entry.trim())).collect()
}

/// The number, as text, of the pull request of an item in `phase`, if it has one.
fn pull_request_number(phase: &ItemPhase) -> Option<String> {
    match phase {
        ItemPhase::InReview { pr, .. }
        | ItemPhase::Fixing { pr, .. }
        | ItemPhase::Approved { pr, .. }
        | ItemPhase::Done {
            completion: Completion::Merged { pr, .. },
        } => Some(pr.number.to_string()),
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::Done {
            completion: Completion::Listed,
        } => None,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod item_tests;

#[cfg(test)]
mod legacy_tests;
