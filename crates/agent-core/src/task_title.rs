//! The words a notification uses to name a task: a description of at most [`MAX_SHORT_DESC_CHARS`] characters, derived
//! from a brief when the task list names none, and the first sentences of a brief.
//!
//! Both are pure: the same brief gives the same text every time, as the rule is one function each, and a text never
//! grows beyond its limit; a cut is marked by [`CUT_MARK`].

/// Longest description of a task, in characters: the line `Short desc:` of a task file and of a notification on a phone.
pub const MAX_SHORT_DESC_CHARS: usize = 70;

/// Longest excerpt of a brief, in characters: what a notification tells after its own text.
pub const MAX_EXCERPT_CHARS: usize = 400;

/// What marks a text cut at its limit; one character, so that a cut text stays within its limit.
pub const CUT_MARK: char = '…';

/// What a brief with no word in it says of its task.
pub const NO_BRIEF_DESC: &str = "a task without a description";

/// What ends a sentence.
const SENTENCE_END: char = '.';

/// The bullet of a list item, which opens a line of a brief but says nothing about the task.
const LIST_BULLET: char = '*';

/// The dash of a list item.
const LIST_DASH: char = '-';

/// The plus of a list item.
const LIST_PLUS: char = '+';

/// The `)` after the number of a numbered list item, as in `1)`.
const NUMBERED_PAREN: char = ')';

/// The byte order mark some editors put at the start of a file; it is not part of its text.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// The description to show for a task whose task list names none: the first sentence of its brief, cut to
/// [`MAX_SHORT_DESC_CHARS`] characters.
#[must_use]
pub fn derived_short_desc(brief: &str) -> String {
    let text = unmarked_text(brief);
    let unmarked = trim_marks(first_sentence(&text));
    let sentence = unmarked.trim_start_matches(BYTE_ORDER_MARK).trim();
    if sentence.is_empty() {
        return NO_BRIEF_DESC.to_owned();
    }
    cut(sentence, MAX_SHORT_DESC_CHARS)
}

/// The first sentences of a brief, at most [`MAX_EXCERPT_CHARS`] characters: the brief whole when it is shorter,
/// its sentences up to the last one that fits, or a cut marked by [`CUT_MARK`].
#[must_use]
pub fn brief_excerpt(brief: &str) -> String {
    let text = brief.trim_start_matches(BYTE_ORDER_MARK).trim();
    if text.chars().count() <= MAX_EXCERPT_CHARS {
        return text.to_owned();
    }
    let fitting = slice_chars(text, MAX_EXCERPT_CHARS).trim_end();
    match fitting.rfind(SENTENCE_END) {
        // The sentences up to the last one that fits; a cut inside the first would only repeat the description.
        Some(at) if !inside_number(fitting, at) && at > 0 => {
            slice_chars(fitting, char_count(fitting, at).saturating_add(1)).to_owned()
        }
        Some(_) | None => cut(fitting, MAX_EXCERPT_CHARS),
    }
}

/// The first line of `brief` with words, without its heading and list marks.
fn unmarked_text(brief: &str) -> String {
    let line = brief.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    trim_marks(line)
}

/// `line` without the heading marks and the list marker that opens it.
fn trim_marks(line: &str) -> String {
    let line = line.trim_start();
    if line.starts_with('#') {
        return line.trim_start_matches('#').trim_start().to_owned();
    }
    let mut chars = line.char_indices();
    let Some((_, first)) = chars.next() else {
        return String::new();
    };
    if matches!(first, LIST_BULLET | LIST_DASH | LIST_PLUS) {
        let rest = line.get(first.len_utf8()..).unwrap_or_default();
        if rest.starts_with(char::is_whitespace) {
            return rest.trim_start().to_owned();
        }
    }
    // A number, its full stop or `)`, then a space: the marker of a numbered list item.
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if let Some(marker) = line.get(digits..).and_then(|rest| rest.chars().next())
        && digits > 0
        && matches!(marker, SENTENCE_END | NUMBERED_PAREN)
        && let Some(rest) = line.get(digits.saturating_add(marker.len_utf8())..)
        && rest.starts_with(char::is_whitespace)
    {
        return rest.trim_start().to_owned();
    }
    line.to_owned()
}

/// The first sentence of `text`: up to and including its first full stop outside a number.
fn first_sentence(text: &str) -> &str {
    let mut from = 0;
    while let Some(found) = text.get(from..).and_then(|rest| rest.find(SENTENCE_END)) {
        let at = from.saturating_add(found);
        if !inside_number(text, at) {
            return slice_chars(text, char_count(text, at).saturating_add(1));
        }
        // The stop of a number such as `v1.2`: the sentence goes on after it.
        from = at.saturating_add(SENTENCE_END.len_utf8());
    }
    text
}

/// Whether the full stop at byte `at` of `text` sits inside a number, as in `v1.2`, rather than ending a sentence.
fn inside_number(text: &str, at: usize) -> bool {
    let before = text.get(..at).and_then(|head| head.chars().next_back());
    let after = text
        .get(at.saturating_add(SENTENCE_END.len_utf8())..)
        .and_then(|rest| rest.chars().next());
    matches!((before, after), (Some(digit), Some(next)) if digit.is_ascii_digit() && next.is_ascii_digit())
}

/// `text` in at most `max` characters; a cut text is marked and says that the text goes on.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut kept = slice_chars(text, max.saturating_sub(1)).trim_end().to_owned();
    kept.push(CUT_MARK);
    kept
}

/// `text` up to `chars` characters.
fn slice_chars(text: &str, chars: usize) -> &str {
    text.char_indices().nth(chars).map_or(text, |(at, _)| text.get(..at).unwrap_or_default())
}

/// How many characters of `text` come before its byte `at`.
fn char_count(text: &str, at: usize) -> usize {
    text.get(..at).unwrap_or_default().chars().count()
}

#[cfg(test)]
mod tests;
