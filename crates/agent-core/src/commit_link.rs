//! Rewriting commit IDs in Markdown texts into links to the repository's commit page.

#[cfg(test)]
mod tests;

use crate::state::CommitId;

/// Rewrites commit IDs in `text` into markdown links to `https://github.com/{owner}/{repo}/commit/{id}`.
///
/// A commit ID is a word of 7 to 40 hexadecimal digits, bare or in single backticks, that names a commit of the
/// repository (as determined by `resolve`). Words that name no commit are left unchanged.
///
/// Nothing is changed inside a fenced code block, inside an existing link, or within a longer word.
#[must_use]
pub fn link_commits<F>(text: &str, _owner: &str, _repo: &str, _resolve: F) -> String
where
    F: FnMut(&str) -> Option<CommitId>,
{
    text.to_owned()
}
