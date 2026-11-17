//! Linking commit hashes in text to full commit identifiers.
//!
//! Given text and a callback to resolve short commit hashes to full IDs, finds and links
//! 7-40 character hex sequences, creating Markdown links. Avoids re-linking inside code
//! fences and existing Markdown links.

/// A region of text that is protected from linking (code fence or existing link).
#[derive(Debug, Clone, Copy)]
struct ProtectedSpan {
    start: usize,
    end: usize,
}

impl ProtectedSpan {
    /// Whether the given position is inside this protected span.
    fn contains(self, pos: usize) -> bool {
        self.start <= pos && pos < self.end
    }
}

/// Checks if a character is a valid hex digit.
fn is_hex_digit(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// Checks if a character is a valid boundary for a hex sequence (not part of an identifier).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Finds all code fence regions (``` or ~~~) in the text.
fn find_code_fences(text: &str) -> Vec<ProtectedSpan> {
    let mut fences = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut in_fence = false;
    let mut fence_start = 0;
    let mut pos = 0;

    for line in lines {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            if in_fence {
                fences.push(ProtectedSpan {
                    start: fence_start,
                    end: pos + line.len(),
                });
                in_fence = false;
            } else {
                fence_start = pos;
                in_fence = true;
            }
        }
        pos += line.len() + 1; // +1 for newline
    }

    fences
}

/// Finds all Markdown link regions [...](...) in the text.
fn find_markdown_links(text: &str) -> Vec<ProtectedSpan> {
    let mut links = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if let Some(&'[') = chars.get(i) {
            let mut j = i + 1;
            let mut depth = 1;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }

            if depth == 0 && j < chars.len() && chars[j] == '(' {
                let mut k = j + 1;
                while k < chars.len() && chars[k] != ')' {
                    k += 1;
                }

                if k < chars.len() && chars[k] == ')' {
                    links.push(ProtectedSpan { start: i, end: k + 1 });
                    i = k + 1;
                    continue;
                }
            }
        }
        i += 1;
    }

    links
}

/// Links commit hashes in text to full commit identifiers.
///
/// Finds 7-40 character hex sequences (short commit hashes) and uses `resolve` to convert
/// them to full commit IDs, creating Markdown links of the form `[short](repo_url/commit/full)`.
/// Avoids linking inside code fences and existing Markdown links.
///
/// # Behavior
/// - Bare hex sequences become `[short](url/commit/full)`
/// - Backtick-enclosed sequences become `[short](url/commit/full)`
/// - Hashes inside code fences are skipped
/// - Existing Markdown links are not relinked
/// - Non-hex longer words are not treated as commits
/// - Hashes outside 7-40 character range are left unchanged
///
/// # Arguments
/// - `text`: the input text to process
/// - `repo_url`: base URL for commits, e.g., `https://github.com/owner/repo`
/// - `resolve`: callback taking a short hash and returning the full commit ID, or None if unresolvable
///
/// # Returns
/// Modified text with resolved commits linked.
pub fn link_commits<F>(text: &str, repo_url: &str, mut resolve: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let protected_spans = {
        let mut spans = find_code_fences(text);
        spans.extend(find_markdown_links(text));
        spans.sort_by_key(|s| s.start);
        spans
    };

    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Check if current position is inside a protected span
        let in_protected = protected_spans.iter().any(|s| s.contains(i));

        if in_protected {
            result.push(chars[i]);
            i += 1;
            continue;
        }

        // Check if we're at the start of a backtick-enclosed sequence
        if let Some(&'`') = chars.get(i)
            && let Some(&first_hex) = chars.get(i + 1)
            && is_hex_digit(first_hex)
        {
            let mut j = i + 1;
            while let Some(&ch) = chars.get(j) {
                if is_hex_digit(ch) {
                    j += 1;
                } else {
                    break;
                }
            }

            let hex_len = j - i - 1;
            if (7..=40).contains(&hex_len)
                && let Some(&'`') = chars.get(j)
            {
                if let Some(hex_slice) = chars.get((i + 1)..j) {
                    let hex_str: String = hex_slice.iter().collect();

                    if let Some(full_hash) = resolve(&hex_str) {
                        result.push('[');
                        result.push_str(&hex_str);
                        result.push(']');
                        result.push('(');
                        result.push_str(repo_url);
                        result.push_str("/commit/");
                        result.push_str(&full_hash);
                        result.push(')');
                        i = j + 1;
                        continue;
                    }
                }
            }
        }

        // Check if we're at the start of a bare hex sequence
        if let Some(&ch) = chars.get(i)
            && is_hex_digit(ch)
        {
            let is_word_start = i == 0 || chars.get(i.saturating_sub(1)).is_none_or(|&c| !is_word_char(c));

            if is_word_start {
                let mut j = i;
                while let Some(&hex_ch) = chars.get(j) {
                    if is_hex_digit(hex_ch) {
                        j += 1;
                    } else {
                        break;
                    }
                }

                let hex_len = j - i;
                let is_word_end = chars.get(j).is_none_or(|&c| !is_word_char(c));

                if (7..=40).contains(&hex_len)
                    && is_word_end
                    && let Some(hex_slice) = chars.get(i..j)
                {
                    let hex_str: String = hex_slice.iter().collect();

                    if let Some(full_hash) = resolve(&hex_str) {
                        result.push('[');
                        result.push_str(&hex_str);
                        result.push(']');
                        result.push('(');
                        result.push_str(repo_url);
                        result.push_str("/commit/");
                        result.push_str(&full_hash);
                        result.push(')');
                        i = j;
                        continue;
                    }
                }
            }
        }

        if let Some(&ch) = chars.get(i) {
            result.push(ch);
        }
        i += 1;
    }

    result
}

#[cfg(test)]
mod tests;
