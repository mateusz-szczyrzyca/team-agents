//! `agent guard-commit`: the `commit-msg` hook of an agent's runs asks whether the commit may be made.
//!
//! The commit is judged by the rule of the checks before a review, asked while it is only staged
//! ([`agent_core::decide::commit_guard`]): a commit that takes lines out of a test a `RED:` commit froze is made as a
//! `TEST-FIX:` commit of test files only, or not at all. The command reads and writes nothing in the repository and asks
//! no human. It judges a commit only in the worktree of a task of this home, for a repository with a `[checks]` table;
//! everywhere else the commit is allowed without a word, and where it cannot judge (a home that does not load, git that
//! fails) it says so and allows it: the checks before the review look at the commit again.

use std::io::{self, Write};
use std::path::Path;

use crate::environment::Environment;

/// What starts a line of the message that the editor shows and git's cleanup removes.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the commit that makes the command work")
)]
const COMMENT: char = '#';

/// The position, after the name of the command, of the parent in `/proc/<pid>/stat`: the state comes first.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the commit that makes the command work")
)]
const PARENT_FIELD: usize = 1;

/// The argument of `git commit` that replaces the last commit.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the commit that makes the command work")
)]
const AMEND: &[u8] = b"--amend";

/// Why `agent guard-commit` failed: the commit is refused, or the answer cannot be written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GuardCommitError {
    /// The commit is refused; the reasons were written.
    #[error("the commit is refused")]
    #[expect(dead_code, reason = "constructed by the commit that makes the command work")]
    Refused,
    /// The answer cannot be written.
    #[error("cannot write the answer of the guard")]
    Output(#[source] io::Error),
}

/// The subject of the message `message`: its first line that is not blank and not a comment of the editor.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the commit that makes the command work")
)]
fn subject_of(message: &str) -> Option<&str> {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with(COMMENT))
}

/// The parent of the process whose `/proc/<pid>/stat` is `stat`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the commit that makes the command work")
)]
fn parent_of(stat: &str) -> Option<u32> {
    // The name of the command is in brackets and may hold brackets: the fields start after the last one.
    let (_, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(PARENT_FIELD)?.parse().ok()
}

/// Whether the command line `cmdline` (arguments separated by NUL, as `/proc/<pid>/cmdline` holds it) asks `git commit`
/// to replace the last commit.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the commit that makes the command work")
)]
fn is_amend(cmdline: &[u8]) -> bool {
    cmdline.split(|byte| *byte == 0).any(|argument| argument == AMEND)
}

/// Asks whether the commit whose message is in `message_file` may be made, in the directory of `environment`.
///
/// # Errors
/// [`GuardCommitError::Refused`] when it may not, after the reasons are written to `out`;
/// [`GuardCommitError::Output`] when `out` refuses the text.
pub(crate) async fn guard_commit(
    _environment: &Environment,
    _message_file: &Path,
    out: &mut impl Write,
) -> Result<(), GuardCommitError> {
    out.flush().map_err(GuardCommitError::Output)
}

#[cfg(test)]
mod tests;
