//! Pure Markdown rewriting of repository commit and file references.

use std::collections::HashSet;
use std::iter::Peekable;
use std::str::Chars;

use crate::state::CommitId;

/// An open fenced code block, closed only by the same marker and sufficient width.
#[derive(Debug, Clone, Copy)]
struct Fence {
    /// Backtick or tilde used by the opening fence.
    marker: char,
    /// Minimum number of markers required to close this block.
    width: usize,
}

/// The Markdown block that must continue for its fenced code to remain open.
#[derive(Debug)]
enum Container {
    /// A blockquote requires a quote marker on each non-lazy code line.
    Quote,
    /// A list item requires this many spaces on its continuation lines.
    List(usize),
}

/// Container ancestry persists across lines; an active fence fixes that ancestry.
#[derive(Debug, Default)]
struct Blocks {
    /// Containers enclosing the current line, in outer-to-inner order.
    containers: Vec<Container>,
    /// An open code fence within exactly these containers.
    active: Option<Fence>,
}

/// Strips at most three spaces, the indentation allowed before a block marker.
fn marker_content(line: &str) -> Option<&str> {
    let content = line.trim_start_matches(' ');
    (line.len().saturating_sub(content.len()) <= 3).then_some(content)
}

/// Matches one existing container without interpreting any new block markers.
fn continuation<'a>(line: &'a str, container: &Container) -> Option<&'a str> {
    match container {
        Container::Quote => {
            let rest = marker_content(line)?.strip_prefix('>')?;
            Some(rest.strip_prefix(' ').unwrap_or(rest))
        }
        Container::List(width) => {
            if line.trim().is_empty() {
                return Some(line);
            }
            let mut rest = line;
            for _ in 0..*width {
                rest = rest.strip_prefix(' ')?;
            }
            Some(rest)
        }
    }
}

/// Reads a list marker and its padding, returning the continuation indentation.
fn list_marker(content: &str) -> Option<(usize, &str)> {
    let mut chars = content.chars();
    let first = chars.next()?;
    let rest = if matches!(first, '-' | '+' | '*') {
        chars.as_str()
    } else if first.is_ascii_digit() {
        let after_digits = content.trim_start_matches(|ch: char| ch.is_ascii_digit());
        if content.len().saturating_sub(after_digits.len()) > 9 {
            return None;
        }
        after_digits
            .strip_prefix('.')
            .or_else(|| after_digits.strip_prefix(')'))?
    } else {
        return None;
    };
    let after_space = rest.strip_prefix(' ')?;
    let trimmed = rest.trim_start_matches(' ');
    let padding = rest.len().saturating_sub(trimmed.len());
    // Five spaces begin indented code, rather than additional marker padding.
    let body = if padding <= 4 { trimmed } else { after_space };
    Some((content.len().saturating_sub(body.len()), body))
}

/// Advances container ancestry, ending a fence when any enclosing block ends.
fn block_content<'a>(line: &'a str, blocks: &mut Blocks) -> &'a str {
    let mut content = line;
    for (depth, container) in blocks.containers.iter().enumerate() {
        if let Some(rest) = continuation(content, container) {
            content = rest;
        } else {
            blocks.containers.truncate(depth);
            blocks.active = None;
            break;
        }
    }
    if blocks.active.is_some() {
        return content;
    }
    while let Some(trimmed) = marker_content(content) {
        if let Some(rest) = trimmed.strip_prefix('>') {
            blocks.containers.push(Container::Quote);
            content = rest.strip_prefix(' ').unwrap_or(rest);
        } else if let Some((width, rest)) = list_marker(trimmed) {
            let indent = content.len().saturating_sub(trimmed.len());
            blocks.containers.push(Container::List(indent.saturating_add(width)));
            content = rest;
        } else {
            break;
        }
    }
    content
}

/// Reads a Markdown fence within its already matched container context.
fn fence(content: &str) -> Option<(Fence, &str)> {
    let content = marker_content(content)?;
    let marker = content.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let rest = content.trim_start_matches(marker);
    let width = content.len().saturating_sub(rest.len());
    (width >= 3).then_some((Fence { marker, width }, rest))
}

/// Tracks fences and returns container-stripped content only outside fenced code.
fn unfenced_content<'a>(line: &'a str, blocks: &mut Blocks) -> Option<&'a str> {
    let content = block_content(line, blocks);
    let delimiter = fence(content);
    if let Some(open) = blocks.active {
        if let Some((closing, rest)) = delimiter
            && closing.marker == open.marker
            && closing.width >= open.width
            && rest.trim().is_empty()
        {
            blocks.active = None;
        }
        None
    } else if let Some((opening, rest)) = delimiter
        && (opening.marker != '`' || !rest.contains('`'))
    {
        blocks.active = Some(opening);
        None
    } else {
        Some(content)
    }
}

/// Links resolved commit words, retaining the author's spelling in code-styled labels.
fn link_words(text: &str, repo_url: &str, resolve: &mut impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
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
    let mut quote = None;
    let mut previous = open;
    while let Some(ch) = chars.next() {
        text.push(ch);
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                text.push(escaped);
            }
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_some() {
            continue;
        } else if open == '(' && previous.is_whitespace() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch == open {
            depth = depth.saturating_add(1);
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return true;
            }
        }
        previous = ch;
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

/// Recognizes an angle-delimited email address without consuming following prose.
fn starts_email(chars: Peekable<Chars<'_>>) -> bool {
    let address: String = chars.take_while(|ch| *ch != '>').collect();
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !address.chars().any(|ch| ch.is_whitespace() || matches!(ch, '<' | '>'))
}

/// Normalizes reference labels according to their case-insensitive whitespace rules.
fn reference_label(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Finds complete reference definitions so shortcuts also work before their definition.
fn references(text: &str) -> HashSet<String> {
    let mut labels = HashSet::new();
    let mut active = Blocks::default();
    for line in text.lines() {
        if let Some(content) = unfenced_content(line, &mut active)
            && let Some(label) = reference_definition(content)
        {
            labels.insert(label);
        }
    }
    labels
}

/// Reads a reference definition's label while preserving its source spelling elsewhere.
fn reference_definition(line: &str) -> Option<String> {
    let content = line.trim_start_matches(' ');
    if line.len().saturating_sub(content.len()) > 3 {
        return None;
    }
    let mut chars = content.strip_prefix('[')?.chars().peekable();
    let mut label = String::new();
    if !balanced(&mut chars, '[', ']', &mut label) || chars.next() != Some(':') {
        return None;
    }
    label.pop()?;
    let destination: String = chars.collect();
    reference_target(destination.trim()).then(|| reference_label(&label))
}

/// Accepts a reference destination and optional title only when the entire line is valid.
fn reference_target(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    if chars.next_if_eq(&'<').is_some() {
        let mut closed = false;
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    chars.next();
                }
                '>' => {
                    closed = true;
                    break;
                }
                '<' | '\n' | '\r' => return false,
                _ => {}
            }
        }
        if !closed {
            return false;
        }
    } else {
        let mut depth = 0_usize;
        let mut consumed = false;
        while let Some(ch) = chars.next_if(|ch| !ch.is_whitespace()) {
            consumed = true;
            match ch {
                '\\' => {
                    chars.next();
                }
                '(' => depth = depth.saturating_add(1),
                ')' if depth == 0 => return false,
                ')' => depth = depth.saturating_sub(1),
                '<' | '>' => return false,
                _ => {}
            }
        }
        if !consumed || depth != 0 {
            return false;
        }
    }
    if chars.peek().is_none() {
        return true;
    }
    if !chars.next().is_some_and(char::is_whitespace) {
        return false;
    }
    while chars.next_if(|ch| ch.is_whitespace()).is_some() {}
    let Some(open) = chars.next() else {
        return true;
    };
    let close = match open {
        '\'' => '\'',
        '"' => '"',
        '(' => ')',
        _ => return false,
    };
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            chars.next();
        } else if ch == close {
            return chars.all(char::is_whitespace);
        }
    }
    false
}

/// Consumes a code span only when an equally wide closing backtick run exists.
fn code_span(chars: &mut Peekable<Chars<'_>>) -> Option<(usize, String, String)> {
    let mut lookahead = chars.clone();
    let mut opening = String::from('`');
    while lookahead.next_if_eq(&'`').is_some() {
        opening.push('`');
    }
    let width = opening.len();
    let mut content = String::new();
    while let Some(ch) = lookahead.next() {
        if ch != '`' {
            content.push(ch);
            continue;
        }
        let mut closing = String::from('`');
        while lookahead.next_if_eq(&'`').is_some() {
            closing.push('`');
        }
        if closing.len() == width {
            let raw = format!("{opening}{content}{closing}");
            *chars = lookahead;
            return Some((width, content, raw));
        }
        content.push_str(&closing);
    }
    None
}

/// URI scheme characters are ASCII and cannot contain Markdown delimiters.
fn scheme_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-')
}

/// Consumes a bare scheme:// URL through its next whitespace boundary.
fn bare_url(first: char, chars: &mut Peekable<Chars<'_>>) -> Option<String> {
    let mut lookahead = chars.clone();
    let mut url = String::from(first);
    while let Some(ch) = lookahead.next_if(|ch| scheme_char(*ch)) {
        url.push(ch);
    }
    if lookahead.next() != Some(':') || lookahead.next() != Some('/') || lookahead.next() != Some('/') {
        return None;
    }
    url.push_str("://");
    while let Some(ch) = lookahead.next_if(|ch| !ch.is_whitespace()) {
        url.push(ch);
    }
    *chars = lookahead;
    Some(url)
}

/// Consumes complete HTML tags or comments, respecting quoted attribute delimiters.
fn html(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
    let mut lookahead = chars.clone();
    let mut raw = String::from('<');
    if lookahead.next_if_eq(&'!').is_some() {
        if lookahead.next() != Some('-') || lookahead.next() != Some('-') {
            return None;
        }
        raw.push_str("!--");
        for ch in lookahead.by_ref() {
            raw.push(ch);
            if raw.ends_with("-->") {
                *chars = lookahead;
                return Some(raw);
            }
        }
        return None;
    }
    if lookahead.next_if_eq(&'/').is_some() {
        raw.push('/');
    }
    if !lookahead.peek().is_some_and(char::is_ascii_alphabetic) {
        return None;
    }
    let mut name = String::new();
    while let Some(ch) = lookahead.next_if(|ch| ch.is_ascii_alphanumeric() || *ch == '-') {
        name.push(ch);
    }
    if !lookahead
        .peek()
        .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '/' | '>'))
    {
        return None;
    }
    // Angle-bracketed commit words remain prose rather than HTML tag names.
    if is_candidate(&name) {
        return None;
    }
    // Preserve the complete original tag after checking its name.
    lookahead = chars.clone();
    raw.truncate(1);
    let mut quote = None;
    for ch in lookahead.by_ref() {
        raw.push(ch);
        match (quote, ch) {
            (Some(open), close) if open == close => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => {
                *chars = lookahead;
                return Some(raw);
            }
            (Some(_) | None, _) => {}
        }
    }
    None
}

/// Protects existing inline and reference links while rewriting ordinary prose.
fn link_prose(text: &str, rewrite: &mut impl FnMut(&str, Style) -> String, references: &HashSet<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prose = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`'
            && let Some((width, content, raw)) = code_span(&mut chars)
        {
            result.push_str(&rewrite(&prose, Style::Bare));
            prose.clear();
            if width == 1 {
                result.push_str(&rewrite(&content, Style::Backticks));
            } else {
                result.push_str(&raw);
            }
            continue;
        }
        let starts_scheme = ch.is_ascii_alphabetic() && !prose.chars().next_back().is_some_and(scheme_char);
        let protected = if starts_scheme {
            bare_url(ch, &mut chars)
        } else if ch == '<' {
            html(&mut chars)
        } else {
            None
        };
        if let Some(raw) = protected {
            result.push_str(&rewrite(&prose, Style::Bare));
            prose.clear();
            result.push_str(&raw);
            continue;
        }
        if ch == '<' && (starts_autolink(chars.clone()) || starts_email(chars.clone())) {
            let mut link = String::from('<');
            let mut closed = false;
            let mut lookahead = chars.clone();
            for next in lookahead.by_ref() {
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
                chars = lookahead;
                result.push_str(&rewrite(&prose, Style::Bare));
                prose.clear();
                result.push_str(&link);
            } else {
                prose.push('<');
            }
            continue;
        }
        if ch != '[' {
            prose.push(ch);
            continue;
        }
        let mut link = String::from('[');
        let mut lookahead = chars.clone();
        let label_closed = balanced(&mut lookahead, '[', ']', &mut link);
        let label_end = lookahead.clone();
        let label = link.clone();
        let destination = lookahead.next_if(|ch| *ch == '(' || *ch == '[');
        if label_closed && let Some(open) = destination {
            link.push(open);
            let close = if open == '(' { ')' } else { ']' };
            if balanced(&mut lookahead, open, close, &mut link) {
                chars = lookahead;
                result.push_str(&rewrite(&prose, Style::Bare));
                prose.clear();
                result.push_str(&link);
                continue;
            }
        }
        if label_closed
            && label
                .strip_prefix('[')
                .and_then(|label| label.strip_suffix(']'))
                .is_some_and(|label| references.contains(&reference_label(label)))
        {
            chars = label_end;
            result.push_str(&rewrite(&prose, Style::Bare));
            prose.clear();
            result.push_str(&label);
            continue;
        }
        // Failed link parsing must leave its contents subject to the same prose protections.
        prose.push('[');
    }
    result.push_str(&rewrite(&prose, Style::Bare));
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
    rewrite_markdown(text, &mut |content, style| match style {
        Style::Bare => link_words(content, repo_url, &mut resolve),
        Style::Backticks => {
            if is_candidate(content)
                && let Some(full) = resolve(content)
            {
                let mut result = String::new();
                push_link(&mut result, content, repo_url, &full);
                result
            } else {
                format!("`{content}`")
            }
        }
    })
}

/// The spelling around a prose token in the original Markdown.
#[derive(Debug, Clone, Copy)]
enum Style {
    /// An ordinary prose word.
    Bare,
    /// An entire single-backtick code span.
    Backticks,
}

/// Runs all reference rewriting through the same Markdown protections.
fn rewrite_markdown(text: &str, rewrite: &mut impl FnMut(&str, Style) -> String) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prose = String::new();
    let mut active = Blocks::default();
    let references = references(text);
    for line in text.split_inclusive('\n') {
        if unfenced_content(line, &mut active).is_none_or(|content| reference_definition(content).is_some()) {
            result.push_str(&link_prose(&prose, rewrite, &references));
            prose.clear();
            result.push_str(line);
        } else {
            prose.push_str(line);
        }
    }
    result.push_str(&link_prose(&prose, rewrite, &references));
    result
}

/// Links repository commit words and file paths in Markdown in one pass.
///
/// `is_file` reports whether a root-relative path is a blob in `commit`'s tree.
/// `resolve_commit` resolves commit words as in [`link_commits`]. File labels
/// retain their original spelling and backticks, with optional line anchors.
/// Existing Markdown links, HTML and fenced code remain unchanged.
#[must_use]
pub fn link_references(
    text: &str,
    repo_url: &str,
    commit: &CommitId,
    resolve_commit: impl FnMut(&str) -> Option<String>,
    is_file: impl FnMut(&str) -> bool,
) -> String {
    link_file_references(text, repo_url, &commit.to_string(), resolve_commit, is_file)
}

/// Shares the walker with the file-only public interface.
pub(crate) fn link_file_references(
    text: &str,
    repo_url: &str,
    commit: &str,
    mut resolve_commit: impl FnMut(&str) -> Option<String>,
    mut is_file: impl FnMut(&str) -> bool,
) -> String {
    rewrite_markdown(text, &mut |content, style| {
        let mut rewrite = |word: &str| {
            file_link(word, repo_url, commit, style, &mut is_file).or_else(|| {
                if is_candidate(word) {
                    resolve_commit(word).map(|full| {
                        let mut result = String::new();
                        push_link(&mut result, word, repo_url, &full);
                        result
                    })
                } else {
                    None
                }
            })
        };
        match style {
            Style::Backticks => match rewrite(content) {
                Some(link) => link,
                None => format!("`{content}`"),
            },
            Style::Bare => rewrite_tokens(content, &mut rewrite),
        }
    })
}

/// Keeps path suffixes together so a path embedded in a larger word is never guessed.
fn token_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | ';' | '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | '`' | '*' | '|' | '='
        )
}

/// Rewrites complete prose tokens while leaving sentence punctuation outside links.
fn rewrite_tokens(text: &str, rewrite: &mut impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if token_separator(ch) {
            result.push(ch);
            continue;
        }
        let mut token = String::from(ch);
        while let Some(next) = chars.next_if(|next| !token_separator(*next)) {
            token.push(next);
        }
        let word = token.trim_end_matches(['.', ':']);
        match rewrite(word) {
            Some(link) => {
                result.push_str(&link);
                if let Some(punctuation) = token.strip_prefix(word) {
                    result.push_str(punctuation);
                }
            }
            None => result.push_str(&token),
        }
    }
    result
}

/// A syntactically valid line suffix, kept as text to preserve its spelling.
fn line_anchor(suffix: &str) -> Option<String> {
    let digits = |word: &str| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_digit());
    if let Some((start, end)) = suffix.split_once('-') {
        (digits(start) && digits(end)).then(|| format!("#L{start}-L{end}"))
    } else {
        digits(suffix).then(|| format!("#L{suffix}"))
    }
}

/// Resolves an exact root-relative file, never a traversal or an absolute path.
fn file_link(
    word: &str,
    repo_url: &str,
    commit: &str,
    style: Style,
    is_file: &mut impl FnMut(&str) -> bool,
) -> Option<String> {
    let (path, anchor) = match word.rsplit_once(':') {
        Some((path, suffix)) => match line_anchor(suffix) {
            Some(anchor) => (path, anchor),
            None => (word, String::new()),
        },
        None => (word, String::new()),
    };
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|part| matches!(part, ".." | "." | ""))
        || !is_file(path)
    {
        return None;
    }
    let label = match style {
        Style::Bare => word.to_owned(),
        Style::Backticks => format!("`{word}`"),
    };
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            for nibble in [byte >> 4, byte & 15] {
                let digit = match nibble {
                    0..=9 => b'0'.saturating_add(nibble),
                    _ => b'A'.saturating_add(nibble.saturating_sub(10)),
                };
                encoded.push(char::from(digit));
            }
        }
    }
    Some(format!("[{label}]({repo_url}/blob/{commit}/{encoded}{anchor})"))
}

#[cfg(test)]
mod tests;
