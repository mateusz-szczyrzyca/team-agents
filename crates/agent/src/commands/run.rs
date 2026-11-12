//! `agent run`: starts a task or a team on the repository the command is run in.
//!
//! The command line names the agents and gives the task: as the last word, or in a file (`-f`). The repository is the
//! one the current directory is in, and its name is the name of its `origin` remote (`[repos.NAME]` of the
//! configuration). The daemon makes the team and its worktrees; this command asks and prints what started and where.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use agent_core::config::{InvalidValue, RepoName};
use agent_core::ids::{AgentName, InvalidName};
use agent_core::protocol::{AgentList, AgentListError};
use agent_core::state::{MergeMode, Team};
use agent_core::task_list::TaskListError;
use agent_runtime::git::GitError;
use agent_runtime::store::Home;
use agent_runtime::text_file::TextFileError;

use crate::client::{ClientError, Connection};

/// What the command line of `agent run` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunInput {
    /// The words that are not options: the agents, and the task when there is no file.
    pub(crate) words: Vec<String>,
    /// The file with the task list, from `-f`.
    pub(crate) file: Option<PathBuf>,
    /// The coordinator asked for, from `--coordinator`.
    pub(crate) coordinator: Option<AgentName>,
    /// What happens to an approved pull request: `--no-merge` stops before the merge.
    pub(crate) merge: MergeMode,
}

/// Why the words of the command line are not agents and a task.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WordsError {
    /// No agent is named.
    #[error("name at least one agent")]
    NoAgents,
    /// The task is missing: neither a last word nor a file.
    #[error("give the task as the last argument, or in a file with -f")]
    NoTask,
    /// A word is not an agent name.
    #[error("{word:?} is not an agent name")]
    Agent {
        /// The word.
        word: String,
        /// What is wrong with it.
        source: InvalidName,
    },
    /// The agents are not a valid list.
    #[error(transparent)]
    Agents(#[from] AgentListError),
}

/// Why `agent run` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunError {
    /// The words of the command line do not make a task.
    #[error(transparent)]
    Words(#[from] WordsError),
    /// The task list file cannot be read.
    #[error("cannot read the task list {path:?}")]
    ReadList {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: TextFileError,
    },
    /// The task list file is not there.
    #[error("the task list {path:?} does not exist")]
    NoListFile {
        /// The file.
        path: PathBuf,
    },
    /// The task is not a task list.
    #[error("the task is not a valid task list")]
    List(#[from] TaskListError),
    /// The directory is in no repository, or git cannot say.
    #[error("cannot find the repository of {directory:?}: run agent in a git checkout")]
    Checkout {
        /// The directory.
        directory: PathBuf,
        /// What git said.
        source: GitError,
    },
    /// The repository has no `origin`.
    #[error("the repository has no remote named origin: its name is the name of origin, as in [repos.NAME]")]
    NoOrigin(#[source] GitError),
    /// The URL of `origin` names no repository.
    #[error("the URL of origin, {url:?}, names no repository")]
    RepoName {
        /// The URL.
        url: String,
        /// What is wrong with the last part of it.
        source: Option<InvalidValue>,
    },
    /// The daemon could not be asked, or refused.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The daemon answered with something else than what was asked.
    #[error("the daemon answered with {kind}, not with the team that started")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
    },
    /// The output cannot be written.
    #[error("cannot write what started")]
    Output(#[source] io::Error),
}

/// The agents and the task the words of a command line say: all words are agents when a file holds the task, else the
/// last word is the task.
///
/// # Errors
/// [`WordsError`].
pub(crate) fn split_words(words: &[String], file_given: bool) -> Result<(AgentList, Option<String>), WordsError> {
    let _ = (words, file_given);
    Err(WordsError::NoAgents)
}

/// The name of the repository whose `origin` is at `url`: the last part of its path, without `.git`.
///
/// # Errors
/// [`RunError::RepoName`] when the URL names nothing, or what it names is not a repository name.
pub(crate) fn repo_name_of(url: &str) -> Result<RepoName, RunError> {
    Err(RunError::RepoName {
        url: url.to_owned(),
        source: None,
    })
}

/// What started, as text: the team, and a line for each of its tasks with where it works.
pub(crate) fn render_started(home: &Home, team: &Team) -> String {
    let _ = (home, team);
    String::new()
}

/// Starts what `input` says in the repository `directory` is in, through `connection`, and writes what started to `out`.
///
/// # Errors
/// [`RunError`].
pub(crate) async fn run(
    connection: &mut Connection,
    home: &Home,
    directory: &Path,
    input: RunInput,
    out: &mut impl Write,
) -> Result<(), RunError> {
    let _ = (connection, home, directory, input, out);
    Ok(())
}

#[cfg(test)]
mod tests;
