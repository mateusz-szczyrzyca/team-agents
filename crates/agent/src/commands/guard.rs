//! `agent guard-commit`: the `commit-msg` hook of an agent's runs asks whether the commit may be made.
//!
//! The commit is judged by the rule of the checks before a review, asked while it is only staged
//! ([`agent_core::decide::commit_guard`]): a commit that takes lines out of a test a `RED:` commit froze is made as a
//! `TEST-FIX:` commit of test files only, or not at all. In a worktree whose plan has `For:` lines every commit also
//! names a step of the plan in a `Step: N.M` trailer, with or without a `[checks]` table. The command reads and writes
//! nothing in the repository and asks no human. It judges a commit only in the worktree of a task of this home; the
//! frozen tests are judged for a repository with a `[checks]` table. Everywhere else the commit is allowed without a
//! word, and where it cannot judge (a home that does not load, git that fails) it says so and allows it: the checks
//! before the review look at the commit again.

use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, CommitGuard, RepoName};
use agent_core::decide::commit_guard::{StepRefusal, StepVerdict, Verdict, judge_step};
use agent_core::plan::{PLAN_FILE, PlanError, parse_plan};
use agent_runtime::commit_guard::text::{RefusalTextError, refusal_text, step_refusal_text};
use agent_runtime::commit_guard::{CommitKind, GuardError, Judged, Operation, judge_staged, operation_in_progress};
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

/// The shortest abbreviation of `--amend` that no other option of `git commit` shares.
const SHORTEST_AMEND: &[u8] = b"--am";

/// The word that ends the options of a command line.
const END_OF_OPTIONS: &[u8] = b"--";

/// The short options of `git commit` that take a value: the message, a file with it, a commit to take it from, a template.
const SHORT_OPTIONS_WITH_VALUE: &[u8] = b"mFCct";

/// The long options of `git commit` that take their value as the next word.
const LONG_OPTIONS_WITH_VALUE: [&str; 12] = [
    "message",
    "file",
    "author",
    "date",
    "template",
    "reuse-message",
    "reedit-message",
    "fixup",
    "squash",
    "cleanup",
    "trailer",
    "pathspec-from-file",
];

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
    let mut arguments = cmdline.split(|byte| *byte == 0);
    while let Some(argument) = arguments.next() {
        if argument == END_OF_OPTIONS {
            return false;
        }
        if is_amend_option(argument) {
            return true;
        }
        if takes_next_argument(argument) {
            // The next word is the value of the option, whatever it looks like.
            arguments.next();
        }
    }
    false
}

/// Whether `argument` is `--amend`, or an abbreviation of it that git accepts: the options of `git commit` that start
/// with `--am` are no other.
fn is_amend_option(argument: &[u8]) -> bool {
    argument.len() >= SHORTEST_AMEND.len() && AMEND.starts_with(argument)
}

/// Whether `argument` is an option of `git commit` whose value is the next word: a long option spelled out, or a cluster
/// of short options that ends with one that takes a value (`-m`, `-am`) and has nothing after it.
fn takes_next_argument(argument: &[u8]) -> bool {
    if let Some(long) = argument.strip_prefix(b"--") {
        return LONG_OPTIONS_WITH_VALUE.iter().any(|name| name.as_bytes() == long);
    }
    let Some(cluster) = argument.strip_prefix(b"-") else {
        return false;
    };
    // The first short option that takes a value takes the rest of the cluster; it takes the next word only when it is last.
    cluster
        .iter()
        .position(|letter| SHORT_OPTIONS_WITH_VALUE.contains(letter))
        .is_some_and(|position| position + 1 == cluster.len())
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
    /// The plan of the task cannot be read.
    #[error("cannot read the plan {path:?}")]
    PlanFile {
        /// The file.
        path: std::path::PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The plan of the task is not a plan.
    #[error(transparent)]
    PlanText(#[from] PlanError),
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

/// Why the commit with `message` is refused for its `Step:` trailer, if it is: in a worktree whose plan has `For:`
/// lines, every commit names a step of the plan. A commit that an operation replays (a cherry-pick, a revert, a rebase)
/// is not written by the agent and is not asked.
async fn step_refusal(git: &Git, directory: &Path, message: &str) -> Result<Option<StepRefusal>, Unjudged> {
    let path = directory.join(PLAN_FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        // A worktree with no plan (a reviewer's, a worker's) has no step to name.
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Unjudged::PlanFile { path, source }),
    };
    let plan = parse_plan(&text)?;
    if !plan.ties_requirements() {
        return Ok(None);
    }
    if matches!(
        operation_in_progress(git).await?,
        Some(Operation::CherryPick | Operation::Revert | Operation::Rebase)
    ) {
        return Ok(None);
    }
    Ok(match judge_step(&plan, message) {
        StepVerdict::Allowed => None,
        StepVerdict::Refused(refusal) => Some(refusal),
    })
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
    if repo.commit_guard != CommitGuard::Enabled {
        return Ok(None);
    }
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
    let step = step_refusal(&git, directory, &message).await?;
    let frozen = match repo.checks.as_ref() {
        Some(rules) => match judge_staged(&git, rules, &repo.base_branch, subject, kind).await? {
            Judged::NotAuthored(_) | Judged::Verdict(Verdict::Allowed) => None,
            Judged::Verdict(Verdict::Refused(refusals)) => Some(refusals),
        },
        None => None,
    };
    if step.is_none() && frozen.is_none() {
        return Ok(None);
    }
    let texts = HarnessPrompts::load(&home.prompts_dir(), settings.config.language()).await?;
    let mut told = Vec::new();
    if let Some(refusals) = frozen {
        told.push(refusal_text(&texts, &refusals)?);
    }
    if let Some(refusal) = step {
        told.push(step_refusal_text(&texts, &refusal)?);
    }
    Ok(Some(told.join("\n")))
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
