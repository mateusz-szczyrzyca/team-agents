//! Pure Markdown rewriting of repository file references with line numbers.

/// Links exact repository file paths, sharing the commit linker's Markdown walk.
///
/// `is_file` reports whether a root-relative path is a blob in `commit`'s tree.
/// File labels retain their spelling, including single backticks and optional
/// line numbers. `repo_url` is the repository HTTPS URL without a trailing slash.
#[must_use]
pub fn link_files(text: &str, repo_url: &str, commit: &str, is_file: impl FnMut(&str) -> bool) -> String {
    crate::commits::link_file_references(text, repo_url, commit, |_| None, is_file)
}

#[cfg(test)]
mod tests;
