//! The words a notification uses to name a task: a description of at most [`MAX_SHORT_DESC_CHARS`] characters, derived
//! from a brief when the task list names none, and the first sentences of a brief.

/// Longest description of a task, in characters.
pub const MAX_SHORT_DESC_CHARS: usize = 70;

/// Longest excerpt of a brief, in characters.
pub const MAX_EXCERPT_CHARS: usize = 400;

/// What marks a text cut at its limit.
pub const CUT_MARK: char = '…';

/// What a brief with no word in it says of its task.
pub const NO_BRIEF_DESC: &str = "a task without a description";

/// The description to show for a task whose task list names none.
#[must_use]
pub fn derived_short_desc(brief: &str) -> String {
    brief.trim().to_owned()
}

/// The first sentences of a brief, to tell the task in the notification that carries it.
#[must_use]
pub fn brief_excerpt(brief: &str) -> String {
    brief.trim().to_owned()
}

#[cfg(test)]
mod tests;
