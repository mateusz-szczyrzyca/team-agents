//! Rewriting commit IDs in Markdown texts into links to the repository's commit page.

#[cfg(test)]
mod tests;

use crate::state::CommitId;

/// Minimum hexadecimal length for an abbreviated commit ID.
const MIN_COMMIT_LEN: usize = 7;

/// Maximum hexadecimal length for a SHA-1 commit ID.
const MAX_COMMIT_LEN: usize = 40;

/// Rewrites commit IDs in `text` into markdown links to `https://github.com/{owner}/{repo}/commit/{id}`.
///
/// A commit ID is a word of 7 to 40 hexadecimal digits, bare or in single backticks, that names a commit of the
/// repository (as determined by `resolve`). Words that name no commit are left unchanged.
///
/// Nothing is changed inside a fenced code block, inside an existing link, or within a longer word.
#[must_use]
pub fn link_commits<F>(text: &str, owner: &str, repo: &str, mut resolve: F) -> String
where
    F: FnMut(&str) -> Option<CommitId>,
{
    let mut result = String::with_capacity(text.len());
    let mut fence_state: Option<(char, usize)> = None;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some((fence_char, min_count)) = fence_state {
            result.push_str(line);
            if is_closing_fence(trimmed, fence_char, min_count) {
                fence_state = None;
            }
        } else if let Some((fence_char, count)) = opening_fence(trimmed) {
            fence_state = Some((fence_char, count));
            result.push_str(line);
        } else {
            transform_line(line, owner, repo, &mut resolve, &mut result);
        }
    }

    result
}

/// Checks if `trimmed` is an opening code fence (at least 3 backticks or tildes).
fn opening_fence(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|&c| c == first).count();
    if count >= 3 { Some((first, count)) } else { None }
}

/// Checks if `trimmed` closes an active fence of character `fence_char` and at least `min_count` length.
fn is_closing_fence(trimmed: &str, fence_char: char, min_count: usize) -> bool {
    let count = trimmed.chars().take_while(|&c| c == fence_char).count();
    count >= min_count && trimmed.chars().skip(count).all(char::is_whitespace)
}

/// Whether `c` is a word constituent that prevents a boundary match.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Rewrites commit IDs within a single non-fenced line.
fn transform_line<F>(line: &str, owner: &str, repo: &str, resolve: &mut F, out: &mut String)
where
    F: FnMut(&str) -> Option<CommitId>,
{
    let mut rest = line;
    let mut prev_char: Option<char> = None;

    while !rest.is_empty() {
        if let Some(link_len) = match_existing_link(rest)
            && let Some(link_str) = rest.get(..link_len)
        {
            out.push_str(link_str);
            prev_char = link_str.chars().last();
            rest = rest.get(link_len..).unwrap_or("");
            continue;
        }

        if let Some((consumed, id_str, in_backticks)) = match_candidate(rest, prev_char) {
            if let Some(full_id) = resolve(id_str) {
                append_link(out, owner, repo, id_str, full_id.as_str(), in_backticks);
            } else if let Some(verbatim) = rest.get(..consumed) {
                out.push_str(verbatim);
            }
            if let Some(consumed_str) = rest.get(..consumed) {
                prev_char = consumed_str.chars().last();
            }
            rest = rest.get(consumed..).unwrap_or("");
            continue;
        }

        if let Some(c) = rest.chars().next() {
            out.push(c);
            prev_char = Some(c);
            rest = rest.get(c.len_utf8()..).unwrap_or("");
        }
    }
}

/// Appends the markdown commit link for `id_str` to `out`.
fn append_link(out: &mut String, owner: &str, repo: &str, id_str: &str, full_id: &str, in_backticks: bool) {
    out.push('[');
    if in_backticks {
        out.push('`');
    }
    out.push_str(id_str);
    if in_backticks {
        out.push('`');
    }
    out.push_str("](https://github.com/");
    out.push_str(owner);
    out.push('/');
    out.push_str(repo);
    out.push_str("/commit/");
    out.push_str(full_id);
    out.push(')');
}

/// Checks if `s` starts with an existing markdown link `[...](...)`, `[...][...]`, or `<http...>`.
fn match_existing_link(s: &str) -> Option<usize> {
    if s.starts_with("<http://") || s.starts_with("<https://") {
        let gt = s.find('>')?;
        return Some(gt.saturating_add(1));
    }

    if s.starts_with('[') {
        let bracket_end = find_matching_bracket(s, '[', ']')?;
        let after_bracket = s.get(bracket_end.saturating_add(1)..)?;
        if after_bracket.starts_with('(') {
            let paren_end = find_matching_bracket(after_bracket, '(', ')')?;
            return Some(
                bracket_end
                    .saturating_add(1)
                    .saturating_add(paren_end)
                    .saturating_add(1),
            );
        }
        if after_bracket.starts_with('[') {
            let ref_end = find_matching_bracket(after_bracket, '[', ']')?;
            return Some(bracket_end.saturating_add(1).saturating_add(ref_end).saturating_add(1));
        }
    }

    None
}

/// Finds the index of the matching closing bracket in `s` (which begins with `open`).
fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth: u32 = 0;
    for (idx, c) in s.char_indices() {
        if c == open {
            depth = depth.saturating_add(1);
        } else if c == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

/// Looks for a candidate commit ID at the start of `s`.
///
/// Returns `Some((consumed_bytes, candidate_hex_str, in_backticks))`.
fn match_candidate(s: &str, prev_char: Option<char>) -> Option<(usize, &str, bool)> {
    if let Some(after_backtick) = s.strip_prefix('`') {
        if after_backtick.starts_with('`') {
            return None;
        }
        let close_pos = after_backtick.find('`')?;
        let inner = after_backtick.get(..close_pos)?;
        if is_commit_hex(inner) {
            let after_close = after_backtick.get(close_pos.saturating_add(1)..)?;
            let next_char = after_close.chars().next();
            if prev_char.is_none_or(|c| !is_word_char(c)) && next_char.is_none_or(|c| !is_word_char(c)) {
                let consumed = close_pos.saturating_add(2);
                return Some((consumed, inner, true));
            }
        }
        return None;
    }

    if prev_char.is_none_or(|c| !is_word_char(c)) {
        let word_len = s.find(|c: char| !is_word_char(c)).unwrap_or(s.len());
        let word = s.get(..word_len)?;
        if is_commit_hex(word) {
            return Some((word_len, word, false));
        }
    }

    None
}

/// Whether `s` is a valid hexadecimal commit ID string of 7 to 40 characters.
fn is_commit_hex(s: &str) -> bool {
    s.len() >= MIN_COMMIT_LEN && s.len() <= MAX_COMMIT_LEN && s.chars().all(|c| c.is_ascii_hexdigit())
}
