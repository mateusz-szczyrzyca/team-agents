//! Pure Markdown rewriting of repository commit references.

use std::iter::Peekable;
use std::str::Chars;

/// An open fenced code block, closed only by the same marker and sufficient width.
#[derive(Debug, Clone, Copy)]
struct Fence {
    /// Backtick or tilde used by the opening fence.
    marker: char,
    /// Minimum number of markers required to close this block.
    width: usize,
}

/// Reads a Markdown fence after at most three leading spaces.
fn fence(line: &str) -> Option<(Fence, &str)> {
    let content = line.trim_start_matches(' ');
    if line.len().saturating_sub(content.len()) > 3 {
        return None;
    }
    let marker = content.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let rest = content.trim_start_matches(marker);
    let width = content.len().saturating_sub(rest.len());
    (width >= 3).then_some((Fence { marker, width }, rest))
}

/// Links resolved commit words, retaining the author's spelling in code-styled labels.
fn link_words(text: &str, repo_url: &str, resolve: &mut impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut lookahead = chars.clone();
            let word = take_word(&mut lookahead);
            if is_candidate(&word) && lookahead.next() == Some('`') {
                // Consume the complete inline code span so its delimiters become the link label.
                chars = lookahead;
                if let Some(full) = resolve(&word) {
                    push_link(&mut result, &word, repo_url, &full);
                } else {
                    result.push('`');
                    result.push_str(&word);
                    result.push('`');
                }
                continue;
            }
        }
        if is_word_char(ch) {
            let mut word = String::from(ch);
            word.push_str(&take_word(&mut chars));
            if is_candidate(&word)
                && let Some(full) = resolve(&word)
            {
                push_link(&mut result, &word, repo_url, &full);
            } else {
                result.push_str(&word);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Identifier characters prevent a hexadecimal substring from becoming a commit word.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Consumes one complete word, including non-ASCII identifier characters.
fn take_word(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut word = String::new();
    while let Some(ch) = chars.next_if(|ch| is_word_char(*ch)) {
        word.push(ch);
    }
    word
}

/// Commit abbreviations have seven through forty ASCII hexadecimal digits.
fn is_candidate(word: &str) -> bool {
    (7..=40).contains(&word.len()) && word.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Appends the Markdown representation of a resolved commit.
fn push_link(result: &mut String, word: &str, repo_url: &str, full: &str) {
    result.push_str("[`");
    result.push_str(word);
    result.push_str("`](");
    result.push_str(repo_url);
    result.push_str("/commit/");
    result.push_str(full);
    result.push(')');
}

/// Consumes a balanced link label or destination, preserving escapes verbatim.
fn balanced(chars: &mut Peekable<Chars<'_>>, open: char, close: char, text: &mut String) -> bool {
    let mut depth = 1_usize;
    while let Some(ch) = chars.next() {
        text.push(ch);
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                text.push(escaped);
            }
        } else if ch == open {
            depth = depth.saturating_add(1);
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return true;
            }
        }
    }
    false
}

/// Markdown autolinks begin with a two-to-thirty-two-character URI scheme.
fn starts_autolink(mut chars: Peekable<Chars<'_>>) -> bool {
    if !chars.next().is_some_and(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    for length in 1..=32 {
        match chars.next() {
            Some(':') => return length >= 2,
            Some(ch) if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-') => {}
            Some(_) | None => return false,
        }
    }
    false
}

/// Protects existing inline and reference links while rewriting ordinary prose.
fn link_prose(text: &str, repo_url: &str, resolve: &mut impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prose = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' && starts_autolink(chars.clone()) {
            let mut link = String::from('<');
            let mut closed = false;
            for next in chars.by_ref() {
                link.push(next);
                if next == '>' {
                    closed = true;
                    break;
                }
                if next.is_whitespace() {
                    break;
                }
            }
            if closed {
                result.push_str(&link_words(&prose, repo_url, resolve));
                prose.clear();
                result.push_str(&link);
            } else {
                prose.push_str(&link);
            }
            continue;
        }
        if ch != '[' {
            prose.push(ch);
            continue;
        }
        result.push_str(&link_words(&prose, repo_url, resolve));
        prose.clear();
        let mut link = String::from('[');
        let label_closed = balanced(&mut chars, '[', ']', &mut link);
        let destination = chars.next_if(|ch| *ch == '(' || *ch == '[');
        if label_closed && let Some(open) = destination {
            link.push(open);
            let close = if open == '(' { ')' } else { ']' };
            if balanced(&mut chars, open, close, &mut link) {
                result.push_str(&link);
                continue;
            }
        }
        result.push_str(&link_words(&link, repo_url, resolve));
    }
    result.push_str(&link_words(&prose, repo_url, resolve));
    result
}

/// Links repository commit words in Markdown without changing fences or existing links.
///
/// `resolve` returns the full commit ID for a seven-to-forty-digit hexadecimal word,
/// or `None` when that word does not name a commit. Bare words and singly backticked
/// words produce code-styled labels preserving their original spelling. `repo_url`
/// is the repository's HTTPS URL, without a trailing slash.
#[must_use]
pub fn link_commits(text: &str, repo_url: &str, mut resolve: impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prose = String::new();
    let mut active: Option<Fence> = None;
    for line in text.split_inclusive('\n') {
        let delimiter = fence(line);
        if let Some(open) = active {
            result.push_str(line);
            if let Some((closing, rest)) = delimiter
                && closing.marker == open.marker
                && closing.width >= open.width
                && rest.trim().is_empty()
            {
                active = None;
            }
        } else if let Some((opening, rest)) = delimiter
            && (opening.marker != '`' || !rest.contains('`'))
        {
            result.push_str(&link_prose(&prose, repo_url, &mut resolve));
            prose.clear();
            result.push_str(line);
            active = Some(opening);
        } else {
            prose.push_str(line);
        }
    }
    result.push_str(&link_prose(&prose, repo_url, &mut resolve));
    result
}

#[cfg(test)]
mod tests;
