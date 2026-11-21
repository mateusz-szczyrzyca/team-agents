//! The words a notification uses to name a task: a description of at most [`MAX_SHORT_DESC_CHARS`] characters,
//! derived from the brief when the task list names none, and an excerpt of the brief.
//!
//! Both are pure functions of the brief: the same brief gives the same text, and a text cut at its limit says so with
//! [`CUT_MARK`].

/// Longest short description of a task, in characters: the line `Short desc:` of a task file and the derived one.
pub const MAX_SHORT_DESC_CHARS: usize = 70;

/// Longest excerpt of a brief, in characters, mark of the cut included.
pub const MAX_EXCERPT_CHARS: usize = 400;

/// What marks a text cut at its limit.
pub const CUT_MARK: char = '…';

/// The description of a task whose brief has no words in it.
pub const NO_BRIEF_DESC: &str = "(no description)";

/// The description to show for a task whose task list names none: the first sentence of `brief`, without its heading
/// or list mark, cut to [`MAX_SHORT_DESC_CHARS`] characters with [`CUT_MARK`].
#[must_use]
pub fn derived_short_desc(_brief: &str) -> String {
    String::new()
}

/// The first sentences of `brief`: the brief whole when it fits in [`MAX_EXCERPT_CHARS`] characters, otherwise its
/// sentences up to the last one that fits, or a cut inside the first, then [`CUT_MARK`].
#[must_use]
pub fn brief_excerpt(_brief: &str) -> String {
    String::from("?")
}

#[cfg(test)]
mod tests;
