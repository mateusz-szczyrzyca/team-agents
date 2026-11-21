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

/// What ends a sentence, when whitespace or the end of the text follows it.
const SENTENCE_END_MARKS: [char; 3] = ['.', '!', '?'];

/// The character that opens a heading line of a brief.
const HEADING_MARK: char = '#';

/// The characters that open an item of a bulleted list.
const BULLETS: [char; 3] = ['-', '*', '+'];

/// The byte order mark some editors put at the start of a file; it is not part of the text.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// The characters of an excerpt that a cut takes for its mark and the space before it.
const EXCERPT_MARK_ROOM: usize = 2;

/// The description to show for a task whose task list names none: the first sentence of `brief`, without its heading
/// or list mark, cut to [`MAX_SHORT_DESC_CHARS`] characters with [`CUT_MARK`].
#[must_use]
pub fn derived_short_desc(brief: &str) -> String {
    let sentence = first_sentence(brief);
    if sentence.is_empty() {
        return NO_BRIEF_DESC.to_owned();
    }
    cut(&sentence, MAX_SHORT_DESC_CHARS)
}

/// The first sentences of `brief`: the brief whole when it fits in [`MAX_EXCERPT_CHARS`] characters, otherwise its
/// sentences up to the last one that fits, or a cut inside the first, then [`CUT_MARK`].
#[must_use]
pub fn brief_excerpt(brief: &str) -> String {
    let text = brief.trim_start_matches(BYTE_ORDER_MARK).trim();
    if text.chars().count() <= MAX_EXCERPT_CHARS {
        return text.to_owned();
    }
    let room = text
        .char_indices()
        .nth(MAX_EXCERPT_CHARS.saturating_sub(EXCERPT_MARK_ROOM))
        .map_or(text.len(), |(at, _)| at);
    match sentence_ends(text).take_while(|end| *end <= room).last() {
        Some(end) => format!("{} {CUT_MARK}", text.get(..end).unwrap_or_default()),
        None => cut(text, MAX_EXCERPT_CHARS),
    }
}

/// The first sentence of `brief` on one line, with the heading or list mark of its first line taken off. A sentence
/// goes on over the following lines of its paragraph until its end, but not into a heading or a list item.
fn first_sentence(brief: &str) -> String {
    let mut lines = brief
        .trim_start_matches(BYTE_ORDER_MARK)
        .lines()
        .map(str::trim)
        .skip_while(|line| line.is_empty());
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut text = strip_mark(first).to_owned();
    for line in lines {
        if line.is_empty() || strip_mark(line) != line || sentence_ends(&text).next().is_some() {
            break;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    let end = sentence_ends(&text).next();
    match end {
        Some(end) => text.get(..end).unwrap_or_default().to_owned(),
        None => text,
    }
}

/// `line` without the heading marks or the list marker that open it; a mark counts only when a space or the end of
/// the line follows it, so `-Fix` and `#tag` stay as they are.
fn strip_mark(line: &str) -> &str {
    let marked = |rest: &'_ str| rest.is_empty() || rest.starts_with(char::is_whitespace);
    let hashes = line.trim_start_matches(HEADING_MARK);
    if hashes.len() < line.len() && marked(hashes) {
        return hashes.trim_start();
    }
    if let Some(rest) = line.strip_prefix(BULLETS)
        && marked(rest)
    {
        return rest.trim_start();
    }
    let digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
    if digits.len() < line.len()
        && let Some(rest) = digits.strip_prefix(['.', ')'])
        && marked(rest)
    {
        return rest.trim_start();
    }
    line
}

/// The byte positions just after each sentence mark in `text` that is followed by whitespace or ends the text:
/// the full stop in `v1.2` does not end a sentence.
fn sentence_ends(text: &str) -> impl Iterator<Item = usize> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while let Some((at, c)) = chars.next() {
            if SENTENCE_END_MARKS.contains(&c) && chars.peek().is_none_or(|(_, next)| next.is_whitespace()) {
                return Some(at.saturating_add(c.len_utf8()));
            }
        }
        None
    })
}

/// `text` in at most `max` characters; a longer text is cut and ends with [`CUT_MARK`] without a space before it.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}{CUT_MARK}", kept.trim_end())
}

#[cfg(test)]
mod tests;
