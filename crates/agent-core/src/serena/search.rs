//! The summary of a CLI's own search (`Grep`, `grep`) or listing (`Glob`, `glob`) of files, as the stream of a run
//! names it, so that [`super::code_search`] can tell from the summary alone what the call searched.
//!
//! The summary is a text: the history keeps it and `agent log` shows it. The rule that decides if a call searches code
//! reads it back, so everything the rule needs is in it — the path, and the filters that limit the search to a kind of
//! file — and a pattern is never taken for a path.

use super::{SEARCH_PATH_MARK, has_text_extension, is_code_file};

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

/// The inputs of a CLI's search or listing the rule needs, as the tool was called.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchInput<'a> {
    /// What the call looks for: the pattern of a `Grep`, the glob of a `Glob`.
    pub pattern: Option<&'a str>,
    /// The file or directory the call was given; none for the whole repository.
    pub path: Option<&'a str>,
    /// The globs that limit a `Grep` to files by name (`*.md`, `*.{ts,tsx}`; `!` first excludes).
    pub globs: Vec<&'a str>,
    /// The names of the file types that limit a `Grep` (`rust`, `md`).
    pub types: Vec<&'a str>,
}

/// The mark that ends the summary of a search of a CLI that is told apart from a pattern: a pattern that ends with
/// [`SEARCH_PATH_MARK`] is given this mark after it, so that it is never read as a path.
const PATTERN_MARK: &str = " (pattern)";

/// The names of the file types of `rg` and of the CLIs that are not the extension of their files.
const TYPE_EXTENSIONS: [(&str, &str); 12] = [
    ("rust", "rs"),
    ("python", "py"),
    ("javascript", "js"),
    ("typescript", "ts"),
    ("ruby", "rb"),
    ("kotlin", "kt"),
    ("csharp", "cs"),
    ("elixir", "ex"),
    ("erlang", "erl"),
    ("perl", "pl"),
    ("markdown", "md"),
    ("shell", "sh"),
];

/// The most alternatives one brace group of a filter is expanded to.
const MAX_BRACE_ALTERNATIVES: usize = 16;

/// The most globs a filter is expanded to, however many brace groups it has: a bound against a filter written to be huge.
const MAX_EXPANDED_GLOBS: usize = 64;

/// The summary of the call of the tool `name` with `input`, cut by `clip` to `max_chars` characters; `None` when `name`
/// is no search or listing, or the call has nothing to summarise.
///
/// A `Grep` without a path or a filter has its pattern as its summary; one that was given a path or a filter has the
/// files it searches, joined by spaces, with [`SEARCH_PATH_MARK`] after them: the path, and each filter that says what
/// kind of file is searched (`*.rs`, `--type md`) under it. A `Glob` has its pattern under its path, a brace group of the pattern (`*.{ts,tsx}`) expanded to its alternatives.
#[must_use]
pub fn search_summary(
    name: &str,
    input: &SearchInput<'_>,
    clip: fn(&str, usize) -> String,
    max_chars: usize,
) -> Option<String> {
    match name {
        "Grep" | "grep" => grep_summary(input, clip, max_chars),
        "Glob" | "glob" => {
            let pattern = input.pattern?;
            let named: Vec<String> = expand_braces(pattern)
                .iter()
                .map(|alternative| under(input.path, alternative))
                .collect();
            Some(clip(&named.join(" "), max_chars))
        }
        _ => None,
    }
}

/// The summary of a `Grep` with `input`.
fn grep_summary(input: &SearchInput<'_>, clip: fn(&str, usize) -> String, max_chars: usize) -> Option<String> {
    let scope = scope_of(input);
    if !scope.is_empty() {
        let room = max_chars.saturating_sub(SEARCH_PATH_MARK.chars().count());
        return Some(format!("{}{SEARCH_PATH_MARK}", clip(&scope.join(" "), room)));
    }
    let pattern = input.pattern?;
    let cut = clip(pattern, max_chars);
    if !cut.ends_with(SEARCH_PATH_MARK) {
        return Some(cut);
    }
    let room = max_chars.saturating_sub(PATTERN_MARK.chars().count());
    Some(format!("{}{PATTERN_MARK}", clip(pattern, room)))
}

/// The files and directories a `Grep` with `input` searches: its path under each of its filters of a known kind, or its
/// path alone when no filter says a kind; nothing for a search of the whole repository.
fn scope_of(input: &SearchInput<'_>) -> Vec<String> {
    let globs = input.globs.iter().flat_map(|glob| expand_braces(glob));
    let types = input.types.iter().map(|kind| {
        let extension = TYPE_EXTENSIONS
            .iter()
            .find_map(|(name, extension)| (name == kind).then_some(*extension))
            .unwrap_or(kind);
        format!("*.{extension}")
    });
    let filters: Vec<String> = globs
        .chain(types)
        .filter(|filter| !filter.starts_with('!') && (is_code_file(filter) || has_text_extension(filter)))
        .map(|filter| under(input.path, &filter))
        .collect();
    if filters.is_empty() {
        input.path.map(str::to_owned).into_iter().collect()
    } else {
        filters
    }
}

/// `pattern` under the directory `path`: as it is for an absolute pattern or with no path.
fn under(path: Option<&str>, pattern: &str) -> String {
    match path {
        Some(path) if !pattern.starts_with('/') => format!("{}/{pattern}", path.trim_end_matches('/')),
        Some(_) | None => pattern.to_owned(),
    }
}

/// The globs a glob with brace groups stands for: `*.{rs,md}` is `*.rs` and `*.md`. A glob without a group, or with an
/// unclosed one, is itself. A group has at most [`MAX_BRACE_ALTERNATIVES`] alternatives and a glob is expanded to at
/// most [`MAX_EXPANDED_GLOBS`] globs; a glob that would give more is kept as the part of it that was reached.
fn expand_braces(glob: &str) -> Vec<String> {
    let mut done = Vec::new();
    let mut pending = vec![glob.to_owned()];
    while let Some(next) = pending.pop() {
        let group = next.split_once('{').and_then(|(before, rest)| {
            let (alternatives, after) = rest.split_once('}')?;
            Some((before.to_owned(), alternatives.to_owned(), after.to_owned()))
        });
        match group {
            Some((before, alternatives, after)) if done.len() + pending.len() < MAX_EXPANDED_GLOBS => {
                pending.extend(
                    alternatives
                        .split(',')
                        .take(MAX_BRACE_ALTERNATIVES)
                        .map(|alternative| format!("{before}{alternative}{after}")),
                );
            }
            Some(_) | None => done.push(next),
        }
    }
    done
}
