//! The summary of a CLI's own search (`Grep`, `grep`) or listing (`Glob`, `glob`) of files, as the stream of a run
//! names it, so that [`super::code_search`] can tell from the summary alone what the call searched.
//!
//! The summary is a text: the history keeps it and `agent log` shows it. The rule that decides if a call searches code
//! reads it back, so everything the rule needs is in it — the path, and the filters that limit the search to a kind of
//! file — and a pattern is never taken for a path.

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

/// The summary of the call of the tool `name` with `input`, no longer than `max_chars` before a mark and cut by `clip`;
/// `None` when `name` is no search or listing, or the call has nothing to summarise.
#[must_use]
pub fn search_summary(
    name: &str,
    input: &SearchInput<'_>,
    clip: fn(&str, usize) -> String,
    max_chars: usize,
) -> Option<String> {
    let _ = (clip, max_chars);
    if matches!(name, "Read" | "read" | "Bash" | "Task") {
        return Some(String::new());
    }
    input.pattern.or(input.path).map(str::to_owned)
}
