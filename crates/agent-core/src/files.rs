//! Pure Markdown rewriting of repository file references with line numbers.

use std::collections::HashSet;
use std::iter::Peekable;
use std::str::Chars;

// Re-export the Blocks, Fence, and related parsing from commits module
// We'll reuse the same fenced code block and container tracking logic

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

/// Identifier characters for path names.
fn is_path_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/')
}

/// Consumes one complete path component.
fn take_path(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut path = String::new();
    while let Some(ch) = chars.next_if(|ch| is_path_char(*ch)) {
        path.push(ch);
    }
    path
}

/// Percent-encodes characters that need encoding in URLs.
fn percent_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '/' => result.push(ch),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

/// Parse line number specification from path (e.g., "file.rs:10" or "file.rs:10-20").
/// Returns (path, line_spec, trailing_punctuation).
fn parse_line_spec(s: &str) -> (String, Option<String>, String) {
    // Strip and track trailing sentence-ending punctuation
    let trimmed = s.trim_end_matches(|c| matches!(c, '.' | ',' | ';' | ')' | ':'));
    let trailing = s[trimmed.len()..].to_string();

    if let Some((path, lines)) = trimmed.rsplit_once(':') {
        // Check if everything after the colon looks like line numbers
        let all_digits_or_dash = lines.chars().all(|c| c.is_ascii_digit() || c == '-');
        if all_digits_or_dash && !lines.is_empty() {
            (path.to_string(), Some(lines.to_string()), trailing)
        } else {
            (trimmed.to_string(), None, trailing)
        }
    } else {
        (trimmed.to_string(), None, trailing)
    }
}

/// Checks if a path looks like a valid repository path (not absolute, no ..).
fn is_valid_path(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.contains("..") && !path.starts_with('-')
}

/// Appends the Markdown representation of a resolved file link.
fn push_file_link(result: &mut String, path: &str, line_spec: Option<&str>, repo_url: &str, commit: &str) {
    let encoded_path = percent_encode(path);
    let url = if let Some(lines) = line_spec {
        // Format line numbers: single line or range
        let formatted_lines = if lines.contains('-') {
            // Range: "10-20" becomes "L10-L20"
            lines
                .split('-')
                .map(|n| format!("L{}", n))
                .collect::<Vec<_>>()
                .join("-")
        } else {
            // Single line: "10" becomes "L10"
            format!("L{}", lines)
        };
        format!("{}/blob/{}/{}#{}", repo_url, commit, encoded_path, formatted_lines)
    } else {
        format!("{}/blob/{}/{}", repo_url, commit, encoded_path)
    };

    result.push('[');
    result.push('`');
    result.push_str(path);
    if let Some(lines) = line_spec {
        result.push(':');
        result.push_str(lines);
    }
    result.push('`');
    result.push(']');
    result.push('(');
    result.push_str(&url);
    result.push(')');
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

/// Normalizes reference labels according to their case-insensitive whitespace rules.
fn reference_label(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
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

/// Links words in prose that might be file paths, retaining the author's spelling in code-styled labels.
fn link_words(text: &str, repo_url: &str, commit: &str, resolve: &mut impl FnMut(&str) -> bool) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if is_path_char(ch) {
            let mut word = String::from(ch);
            word.push_str(&take_path(&mut chars));

            let (path, line_spec, trailing) = parse_line_spec(&word);
            if is_valid_path(&path) && resolve(&path) {
                push_file_link(&mut result, &path, line_spec.as_deref(), repo_url, commit);
                result.push_str(&trailing);
            } else {
                result.push_str(&word);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Protects existing inline and reference links while rewriting ordinary prose.
fn link_prose(
    text: &str,
    repo_url: &str,
    commit: &str,
    resolve: &mut impl FnMut(&str) -> bool,
    references: &HashSet<String>,
) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prose = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`'
            && let Some((width, content, raw)) = code_span(&mut chars)
        {
            result.push_str(&link_words(&prose, repo_url, commit, resolve));
            prose.clear();
            if width == 1 {
                let (path, line_spec, _trailing) = parse_line_spec(&content);
                if is_valid_path(&path) && resolve(&path) {
                    push_file_link(&mut result, &path, line_spec.as_deref(), repo_url, commit);
                } else {
                    result.push_str(&raw);
                }
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
            result.push_str(&link_words(&prose, repo_url, commit, resolve));
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
                result.push_str(&link_words(&prose, repo_url, commit, resolve));
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
                result.push_str(&link_words(&prose, repo_url, commit, resolve));
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
            result.push_str(&link_words(&prose, repo_url, commit, resolve));
            prose.clear();
            result.push_str(&label);
            continue;
        }
        // Failed link parsing must leave its contents subject to the same prose protections.
        prose.push('[');
    }
    result.push_str(&link_words(&prose, repo_url, commit, resolve));
    result
}

/// Links repository file paths in Markdown without changing fences or existing links.
///
/// `resolve` returns `true` when a path names a file in the commit, `false` when
/// it does not name a file (directory, missing path, etc.). The function treats
/// invalid paths (absolute, containing `..`) as non-files. `repo_url` is the
/// repository's HTTPS URL without a trailing slash, and `commit` is the commit ID
/// (full or short) to include in links.
#[must_use]
pub fn link_files(text: &str, repo_url: &str, commit: &str, mut resolve: impl FnMut(&str) -> bool) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prose = String::new();
    let mut active = Blocks::default();
    let references = references(text);
    for line in text.split_inclusive('\n') {
        if unfenced_content(line, &mut active).is_none_or(|content| reference_definition(content).is_some()) {
            result.push_str(&link_prose(&prose, repo_url, commit, &mut resolve, &references));
            prose.clear();
            result.push_str(line);
        } else {
            prose.push_str(line);
        }
    }
    result.push_str(&link_prose(&prose, repo_url, commit, &mut resolve, &references));
    result
}

#[cfg(test)]
mod tests;
