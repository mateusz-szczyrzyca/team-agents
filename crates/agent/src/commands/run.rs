//! `agent run`: starts a task or a team on the repository the command is run in.
//!
//! The command line names the agents and gives the task: as the last word, or in a file (`-f`). The repository is the
//! one the current directory is in, and its name is the name of its `origin` remote (`[repos.NAME]` of the
//! configuration). The daemon makes the team and its worktrees; this command asks and prints what started and where.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue, RepoName};
use agent_core::ids::{AgentName, InvalidName};
use agent_core::protocol::{AgentList, AgentListError, Client, Command, Reply, RunRequest, ShowRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, TaskListText, Team};
use agent_core::task_list::{MAX_TASK_LIST_BYTES, TaskListError};
use agent_runtime::git::{CommitIdentity, Git, GitError};
use agent_runtime::store::Home;
use agent_runtime::text_file::{TextFileError, read_text_file};

use super::status::kind_of;
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
    /// The current directory is not an absolute path of UTF-8 text.
    #[error("the directory {directory:?} cannot be used")]
    Directory {
        /// The directory.
        directory: PathBuf,
        /// What is wrong with it, if the text was refused.
        source: Option<InvalidValue>,
    },
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
    let (agent_words, task) = if file_given {
        (words, None)
    } else {
        match words.split_last() {
            Some((task, agents)) => (agents, Some(task.clone())),
            None => return Err(WordsError::NoAgents),
        }
    };
    if agent_words.is_empty() {
        return Err(if task.is_some() {
            WordsError::NoTask
        } else {
            WordsError::NoAgents
        });
    }
    let agents = agent_words
        .iter()
        .map(|word| {
            word.parse::<AgentName>().map_err(|source| WordsError::Agent {
                word: word.clone(),
                source,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((AgentList::try_from(agents)?, task))
}

/// The name of the repository whose `origin` is at `url`: the last part of its path, without `.git`.
///
/// # Errors
/// [`RunError::RepoName`] when the URL names nothing, or what it names is not a repository name.
pub(crate) fn repo_name_of(url: &str) -> Result<RepoName, RunError> {
    // The path of the URL: after the authority of `scheme://authority/path`, after the host of `host:path`.
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/').map_or("", |(_, path)| path)
    } else if let Some((_, path)) = url.split_once(':') {
        path
    } else {
        url
    };
    let trimmed = path.trim_end_matches('/');
    let last = trimmed.rsplit_once('/').map_or(trimmed, |(_, last)| last);
    let name = last.strip_suffix(GIT_SUFFIX).unwrap_or(last);
    if name.is_empty() {
        return Err(RunError::RepoName {
            url: url.to_owned(),
            source: None,
        });
    }
    name.parse().map_err(|source| RunError::RepoName {
        url: url.to_owned(),
        source: Some(source),
    })
}

/// What the name of a repository's directory on a server ends with.
const GIT_SUFFIX: &str = ".git";

/// What started, as text: the team, and a line for each of its tasks with where it works.
pub(crate) fn render_started(home: &Home, team: &Team) -> String {
    let roster = team.crew().roster();
    let role_width = roster.iter().map(|(role, _)| role.to_string().len()).max().unwrap_or(0);
    let agent_width = roster
        .iter()
        .map(|(_, member)| member.agent().name().len())
        .max()
        .unwrap_or(0);
    let mut lines = vec![format!(
        "team {} started on {}, merge: {}\n",
        team.id(),
        team.repo(),
        team.merge().as_str()
    )];
    for (role, member) in roster {
        let item = team.items().iter().find_map(|progress| match progress.phase() {
            ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. }
                if assignee.as_str() == member.agent().name() =>
            {
                Some(progress.id())
            }
            ItemPhase::Pending
            | ItemPhase::InProgress { .. }
            | ItemPhase::InReview { .. }
            | ItemPhase::Fixing { .. }
            | ItemPhase::Approved { .. }
            | ItemPhase::Done { .. } => None,
        });
        let with_item = item.map_or_else(String::new, |item| format!("  item {item}"));
        lines.push(format!(
            "  {}  {:<role_width$}  {:<agent_width$}  {}{with_item}\n",
            member.task(),
            role.to_string(),
            member.agent().name(),
            home.worktree_dir(team.repo(), member.task()).display(),
        ));
    }
    lines.concat()
}

/// The request that starts what `input` says in the repository `directory` is in: nothing is asked of the daemon yet.
///
/// # Errors
/// [`RunError`] for words that make no task, a task list that cannot be read or parsed, and a directory that is in no
/// repository or in one without an `origin`.
pub(crate) async fn prepare(directory: &Path, input: RunInput) -> Result<RunRequest, RunError> {
    let (agents, inline) = split_words(&input.words, input.file.is_some())?;
    let text = match (&input.file, inline) {
        (Some(path), _) => read_list(path).await?,
        (None, Some(text)) => text,
        (None, None) => return Err(WordsError::NoTask.into()),
    };
    let task_list = TaskListText::new(text)?;
    let (checkout, repo) = repository(directory).await?;
    Ok(RunRequest {
        repo,
        checkout,
        agents,
        task_list,
        coordinator: input.coordinator,
        merge: input.merge,
    })
}

/// Asks the daemon, through `connection`, to start `request`, and writes what started to `out`.
///
/// # Errors
/// [`RunError`].
pub(crate) async fn start(
    connection: &mut Connection,
    home: &Home,
    request: RunRequest,
    out: &mut impl Write,
) -> Result<(), RunError> {
    let reply = connection.request(Client::Cli, Command::Run(request)).await?;
    let Reply::Started { team, .. } = reply else {
        return Err(RunError::UnexpectedReply { kind: kind_of(&reply) });
    };
    let shown = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Team(team),
            }),
        )
        .await?;
    let Reply::Team { team } = shown else {
        return Err(RunError::UnexpectedReply { kind: kind_of(&shown) });
    };
    out.write_all(render_started(home, &team).as_bytes())
        .map_err(RunError::Output)?;
    out.flush().map_err(RunError::Output)
}

/// The text of the task list file at `path`.
async fn read_list(path: &Path) -> Result<String, RunError> {
    match read_text_file(path, MAX_TASK_LIST_BYTES).await {
        Ok(Some(file)) => Ok(file.text),
        Ok(None) => Err(RunError::NoListFile {
            path: path.to_path_buf(),
        }),
        Err(source) => Err(RunError::ReadList {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The root of the checkout `directory` is in, and the name of its repository.
async fn repository(directory: &Path) -> Result<(AbsolutePath, RepoName), RunError> {
    let text = directory.to_str().ok_or_else(|| RunError::Directory {
        directory: directory.to_path_buf(),
        source: None,
    })?;
    let here = AbsolutePath::from_str(text).map_err(|source| RunError::Directory {
        directory: directory.to_path_buf(),
        source: Some(source),
    })?;
    // Only questions are asked of git: no commit is made, so the identity is a placeholder.
    let git = Git::new(
        here,
        CommitIdentity::new(IDENTITY_NAME.to_owned(), IDENTITY_EMAIL.to_owned(), None),
    );
    let root = git.toplevel().await.map_err(|source| RunError::Checkout {
        directory: directory.to_path_buf(),
        source,
    })?;
    let url = git.origin_url().await.map_err(RunError::NoOrigin)?;
    Ok((root, repo_name_of(&url)?))
}

/// The name of the placeholder identity of the questions this command asks git.
const IDENTITY_NAME: &str = "agent";

/// The e-mail of the placeholder identity.
const IDENTITY_EMAIL: &str = "agent@localhost.invalid";

#[cfg(test)]
mod tests;
