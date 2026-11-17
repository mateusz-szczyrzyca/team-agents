//! Pure Markdown rewriting of repository file references with line numbers.

use crate::state::CommitId;

/// Links exact repository file paths, sharing the commit linker's Markdown walk.
///
/// `is_file` reports whether a root-relative path is a blob in `commit`'s tree.
/// File labels retain their spelling, including single backticks and optional
/// line numbers. `repo_url` is the repository HTTPS URL without a trailing slash.
///
/// The publication revision must be a validated full commit ID:
///
/// ```
/// use agent_core::{files::link_files, state::CommitId};
/// # fn main() -> Result<(), agent_core::state::InvalidCommitId> {
/// let commit: CommitId = "0123456789012345678901234567890123456789".parse()?;
/// assert_eq!(
///     link_files("src/lib.rs", "https://github.com/owner/repo", &commit, |path| path == "src/lib.rs"),
///     format!("[src/lib.rs](https://github.com/owner/repo/blob/{commit}/src/lib.rs)"),
/// );
/// # Ok(())
/// # }
/// ```
///
/// A string cannot be substituted for a publication commit:
///
/// ```compile_fail
/// use agent_core::files::link_files;
/// link_files("src/lib.rs", "https://github.com/owner/repo", "main", |_| true);
/// ```
#[must_use]
pub fn link_files(text: &str, repo_url: &str, commit: &CommitId, is_file: impl FnMut(&str) -> bool) -> String {
    crate::commits::link_file_references(text, repo_url, commit, |_| None, is_file)
}

#[cfg(test)]
mod tests;
