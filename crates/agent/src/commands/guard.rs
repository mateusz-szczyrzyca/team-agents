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
use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::decide::commit_guard::Verdict;
use agent_runtime::commit_guard::text::{RefusalTextError, refusal_text};
use agent_runtime::commit_guard::{CommitKind, GuardError, Judged, judge_staged};
use agent_runtime::git::{CommitIdentity, Git};
use agent_runtime::harness_prompts::{HarnessPrompts, HarnessPromptsError};
use agent_runtime::process::error_chain;
use agent_runtime::settings::{Settings, SettingsError};

use super::gate::worktree_in;
use crate::environment::{Environment, EnvironmentError};

/// The name of the identity git is run with here: no commit is made, but a handle has one.
const READER_NAME: &str = "team-agents";

/// The e-mail of that identity.
const READER_EMAIL: &str = "team-agents@team-agents.invalid";

/// What starts a line of the message that the editor shows and git's cleanup removes.
const COMMENT: char = '#';

/// The position, after the name of the command, of the parent in `/proc/<pid>/stat`: the state comes first.
const PARENT_FIELD: usize = 1;

/// The argument of `git commit` that replaces the last commit.
const AMEND: &[u8] = b"--amend";

/// Why `agent guard-commit` failed: the commit is refused, or the answer cannot be written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GuardCommitError {
    /// The commit is refused; the reasons were written.
    #[error("the commit is refused")]
    Refused,
    /// The answer cannot be written.
    #[error("cannot write the answer of the guard")]
    Output(#[source] io::Error),
}

/// The subject of the message `message`: its first line that is not blank and not a comment of the editor.
fn subject_of(message: &str) -> Option<&str> {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with(COMMENT))
}

/// The parent of the process whose `/proc/<pid>/stat` is `stat`.
fn parent_of(stat: &str) -> Option<u32> {
    // The name of the command is in brackets and may hold brackets: the fields start after the last one.
    let (_, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(PARENT_FIELD)?.parse().ok()
}

/// Whether the command line `cmdline` (arguments separated by NUL, as `/proc/<pid>/cmdline` holds it) asks `git commit`
/// to replace the last commit.
fn is_amend(cmdline: &[u8]) -> bool {
    cmdline.split(|byte| *byte == 0).any(|argument| argument == AMEND)
}

/// The most ancestors of this process that are looked at for `git commit --amend`: the hook is run by git, which is run
/// by the shell of the agent, a few steps below the CLI.
const MAX_ANCESTORS: usize = 8;

/// Why the commit cannot be judged.
#[derive(Debug, thiserror::Error)]
enum Unjudged {
    /// `AGENTS_HOME` or the directory of the process cannot be used.
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    /// The settings of the home cannot be read.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// The worktree is of a repository that the configuration does not have.
    #[error("the configuration has no repository {repository}")]
    Repository {
        /// The repository the worktree belongs to.
        repository: RepoName,
    },
    /// The directory of the process is not text.
    #[error("the directory {path:?} is not text")]
    DirectoryNotText {
        /// The directory.
        path: std::path::PathBuf,
    },
    /// The directory of the process is no path git can be given.
    #[error("the directory {path:?} cannot be used")]
    Directory {
        /// The directory.
        path: std::path::PathBuf,
        /// Why not.
        #[source]
        source: agent_core::config::InvalidValue,
    },
    /// The file with the message of the commit cannot be read.
    #[error("cannot read the message {path:?}")]
    Message {
        /// The file git gave.
        path: std::path::PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The texts of the harness cannot be loaded.
    #[error(transparent)]
    Texts(#[from] HarnessPromptsError),
    /// Git or the checkout cannot be read.
    #[error(transparent)]
    Guard(#[from] GuardError),
    /// The text of the refusal cannot be made.
    #[error(transparent)]
    Refusal(#[from] RefusalTextError),
}

/// Whether a process above this one is `git commit --amend`: the commit then replaces the last one. A process that
/// cannot be read ends the search with no amend found, and the commit is judged as a new one, which is the stricter
/// reading.
async fn amend_requested() -> bool {
    let mut pid = std::process::id();
    for _ in 0..MAX_ANCESTORS {
        let stat = match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
            Ok(stat) => stat,
            Err(error) => {
                tracing::debug!(%error, pid, "a process of the ancestry cannot be read");
                return false;
            }
        };
        let Some(parent) = parent_of(&stat).filter(|parent| *parent > 1) else {
            return false;
        };
        match tokio::fs::read(format!("/proc/{parent}/cmdline")).await {
            Ok(cmdline) if is_amend(&cmdline) => return true,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(%error, pid = parent, "a process of the ancestry cannot be read");
                return false;
            }
        }
        pid = parent;
    }
    false
}

/// The text that tells why the commit may not be made, or nothing when it may.
async fn judge(environment: &Environment, message_file: &Path) -> Result<Option<String>, Unjudged> {
    let home = environment.home()?;
    let directory = environment.directory()?;
    let Some((repository, _task)) = worktree_in(&home, directory).await else {
        return Ok(None);
    };
    let settings = Settings::load(&home).await?;
    let Some(repo) = settings.config.repo(&repository) else {
        return Err(Unjudged::Repository { repository });
    };
    let Some(rules) = repo.checks.as_ref() else {
        return Ok(None);
    };
    let message = tokio::fs::read_to_string(message_file)
        .await
        .map_err(|source| Unjudged::Message {
            path: message_file.to_path_buf(),
            source,
        })?;
    let Some(subject) = subject_of(&message) else {
        return Ok(None);
    };
    let kind = if amend_requested().await {
        CommitKind::Amend
    } else {
        CommitKind::New
    };
    let invalid = |source| Unjudged::Directory {
        path: directory.to_path_buf(),
        source,
    };
    let text = directory.to_str().ok_or_else(|| Unjudged::DirectoryNotText {
        path: directory.to_path_buf(),
    })?;
    // Nothing is committed through this handle: it only reads, and a handle has an identity.
    let git = Git::new(
        AbsolutePath::from_str(text).map_err(invalid)?,
        CommitIdentity::new(READER_NAME.to_owned(), READER_EMAIL.to_owned(), None),
    );
    match judge_staged(&git, rules, &repo.base_branch, subject, kind).await? {
        Judged::NotAuthored(_) | Judged::Verdict(Verdict::Allowed) => Ok(None),
        Judged::Verdict(Verdict::Refused(refusals)) => {
            let texts = HarnessPrompts::load(&home.prompts_dir(), settings.config.language()).await?;
            Ok(Some(refusal_text(&texts, &refusals)?))
        }
    }
}

/// Asks whether the commit whose message is in `message_file` may be made, in the directory of `environment`.
///
/// # Errors
/// [`GuardCommitError::Refused`] when it may not, after the reasons are written to `out`;
/// [`GuardCommitError::Output`] when `out` refuses the text.
pub(crate) async fn guard_commit(
    environment: &Environment,
    message_file: &Path,
    out: &mut impl Write,
) -> Result<(), GuardCommitError> {
    match judge(environment, message_file).await {
        Ok(None) => Ok(()),
        Ok(Some(text)) => {
            writeln!(out, "{text}").map_err(GuardCommitError::Output)?;
            Err(GuardCommitError::Refused)
        }
        // A guard that cannot judge says so and lets the commit be made: the checks before the review look at it again.
        Err(reason) => writeln!(
            out,
            "agent guard-commit: this commit was not judged ({}); it is made, and the checks before the review will look at it",
            error_chain(&reason)
        )
        .map_err(GuardCommitError::Output),
    }
}

#[cfg(test)]
mod tests;
