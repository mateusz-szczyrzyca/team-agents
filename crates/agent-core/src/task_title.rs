//! The words a notification uses to name a task: a description of at most [`MAX_SHORT_DESC_CHARS`] characters, derived
//! from a brief when the task list names none, and the first sentences of a brief.
//!
//! Both are pure: the same brief gives the same text every time, as the rule is one function each, and a text never
//! grows beyond its limit, a cut is marked by [`CUT_MARK`].

/// Longest description of a task, in characters: the line `Short desc:` of a task file and of a notification on a phone.
pub const MAX_SHORT_DESC_CHARS: usize = 70;

/// Longest excerpt of a brief, in characters: what a notification tells after its own text.
pub const MAX_EXCERPT_CHARS: usize = 400;

/// What marks a text cut at its limit; one character, so that a cut text stays within its limit.
pub const CUT_MARK: char = '…';

/// What a brief with no word in it says of its task.
pub const NO_BRIEF_DESC: &str = "a task without a description";

/// The marks of a heading that open a line of a brief but say nothing about the task.
const HEADING_MARKS: &str = "#";

/// The marks of a list item that open a line of a brief but say nothing about the task.
const LIST_MARKS: &str = "-*+";

/// The description to show for a task whose task list names none.
#[must_use]
pub fn derived_short_desc(brief: &str) -> String {
    derived_short_desc_in(brief, MAX_SHORT_DESC_CHARS)
}

/// The first sentences of a brief, to tell the task in the notification that carries it.
#[must_use]
pub fn brief_excerpt(brief: &str) -> String {
    brief_excerpt_in(brief, MAX_EXCERPT_CHARS)
}

/// [`derived_short_desc`] with the limit `max`.
fn derived_short_desc_in(brief: &str, max: usize) -> String {
    let text = without_marks(brief);
    let sentence = first_sentence(&text);
    if sentence.trim().is_empty() {
        return NO_BRIEF_DESC.to_owned();
    }
    cut(sentence.trim(), max)
}

/// [`brief_excerpt`] with the limit `max`.
fn brief_excerpt_in(brief: &str, max: usize) -> String {
    let text = brief.trim();
    if text.is_empty() {
        return String::new();
    }
    let fits = without_marks_up_to(text, max);
    let trimmed = fits.trim_end();
    // The brief goes on past the limit: stop at the last sentence end that fits, or inside the sentence, marked.
    let ends_whole = trimmed.chars().count() == text.chars().count();
    if ends_whole {
        return trimmed.to_owned();
    }
    match trimmed.rfind(SENTENCE_END) {
        Some(at) => trimmed.get(..at.saturating_add(SENTENCE_END.len_utf8())).map_or_else(
            || cut(trimmed, max),
            |ended| {
                // A cut inside the first sentence would only repeat the description.
                if ended.trim().is_empty() { cut(trimmed, max) } else { ended.to_owned() }
            },
        ),
        None => cut(trimmed, max),
    }
}

/// What ends a sentence in a brief.
const SENTENCE_END: char = '.';

/// `brief` without its leading blank lines, heading marks and list marks.
fn without_marks(brief: &str) -> String {
    let first = brief.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    first.trim_start_matches(|c: char| HEADING_MARKS.contains(c) || LIST_MARKS.contains(c) || c.is_whitespace()).to_owned()
}

/// `text` up to `max` characters.
fn without_marks_up_to(text: &str, max: usize) -> &str {
    text.char_indices().nth(max).map_or(text, |(at, _)| text.get(..at).unwrap_or(text))
}

/// The first sentence of `text`.
fn first_sentence(text: &str) -> &str {
    text.find(SENTENCE_END).and_then(|at| text.get(..at.saturating_add(SENTENCE_END.len_utf8()))).unwrap_or(text)
}

/// `text` in at most `max` characters; a cut text is marked and says that the text goes on.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(max.saturating_sub(1)).collect();
    kept.truncate(kept.trim_end().len());
    kept.push(CUT_MARK);
    kept
}

#[cfg(test)]
mod tests;
