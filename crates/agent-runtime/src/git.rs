//! Typed git operations over the `git` command: worktrees, branches, merges without fast-forward, the state of the
//! tree, revisions and push.
//!
//! Every operation builds its argument vector from typed values (a [`BranchName`] cannot start with `-`, a
//! [`Revision`] is a branch, a commit or `HEAD`), runs `git` through the bounded capture of [`crate::process`] and
//! reads its answer into a type. What an operation reports as an answer (a merge that conflicts, a branch that does
//! not exist) is a value; everything else git refuses is a [`GitError`] with git's own message.
//!
//! The commit identity and the signing of a run belong to its [`Git`] handle and travel with every command as
//! environment and `-c` options: nothing is written to a git configuration file.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::string::FromUtf8Error;
use std::time::Duration;

use agent_core::config::{AbsolutePath, EnvName, GitIdentity};
use agent_core::ids::{AgentName, BranchName};
use agent_core::state::{CommitId, InvalidCommitId};
use tokio::process::Command;

use crate::process::{CaptureError, CaptureLimits, Captured, ExitReport, Stream, capture};

/// How long a local git command may run before its group is killed.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a command that talks to a remote may run.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(600);

/// How many bytes of the output of a command are kept.
const MAX_OUTPUT_BYTES: u64 = 1 << 20;

/// How many characters of git's message an error carries.
const MAX_DIAGNOSTIC_CHARS: usize = 2048;

/// The remote branches are pushed to.
const REMOTE: &str = "origin";

/// The commits an agent makes: who they are by, and the key that signs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitIdentity {
    /// The name of the author and the committer.
    name: String,
    /// The e-mail of the author and the committer.
    email: String,
    /// The SSH private key that signs the commits; without one they are unsigned.
    signing_key: Option<AbsolutePath>,
}

impl CommitIdentity {
    /// An identity with the given name and e-mail.
    #[must_use]
    pub fn new(name: String, email: String, signing_key: Option<AbsolutePath>) -> Self {
        Self {
            name,
            email,
            signing_key,
        }
    }

    /// The identity `agent`'s commits have under the `[git]` section of the configuration.
    #[must_use]
    pub fn for_agent(config: &GitIdentity, agent: &AgentName) -> Self {
        Self::new(
            config.name.render(agent),
            config.email.render(agent),
            config.signing_key.clone(),
        )
    }
}

/// What a command may take: its time, and the bytes of its output that are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitLimits {
    /// The time of a command that works on the local repository.
    pub local_timeout: Duration,
    /// The time of a command that talks to a remote.
    pub network_timeout: Duration,
    /// The bytes kept of the output of a command; an answer longer than that is an error.
    pub max_output_bytes: u64,
}

impl Default for GitLimits {
    fn default() -> Self {
        Self {
            local_timeout: LOCAL_TIMEOUT,
            network_timeout: NETWORK_TIMEOUT,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

/// A point in the history that an operation starts from or asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revision {
    /// What the checkout has out now.
    Head,
    /// The tip of a branch.
    Branch(BranchName),
    /// A commit.
    Commit(CommitId),
}

impl fmt::Display for Revision {
    /// The revision as git is given it; a branch is spelled out as its reference so that a tag of the same name
    /// cannot stand in for it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Head => f.write_str("HEAD"),
            Self::Branch(branch) => write!(f, "refs/heads/{branch}"),
            Self::Commit(commit) => write!(f, "{commit}"),
        }
    }
}

/// What git was asked to do, for an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum GitOperation {
    /// `git worktree add`.
    #[strum(serialize = "worktree add")]
    WorktreeAdd,
    /// `git worktree remove`.
    #[strum(serialize = "worktree remove")]
    WorktreeRemove,
    /// `git branch <name>`.
    #[strum(serialize = "branch create")]
    BranchCreate,
    /// `git branch -d`.
    #[strum(serialize = "branch delete")]
    BranchDelete,
    /// `git rev-parse` of a revision.
    #[strum(serialize = "rev-parse")]
    RevParse,
    /// `git merge --no-ff`.
    #[strum(serialize = "merge")]
    Merge,
    /// `git merge --abort`.
    #[strum(serialize = "merge abort")]
    MergeAbort,
    /// `git rev-parse MERGE_HEAD`, to see whether a merge is waiting for its conflicts.
    #[strum(serialize = "merge state")]
    MergeState,
    /// `git status`.
    #[strum(serialize = "status")]
    Status,
    /// `git push`.
    #[strum(serialize = "push")]
    Push,
}

/// Why a git operation failed.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The command could not be run to its end: not started, over its time, or its output could not be read.
    #[error("cannot run git {operation}")]
    Run {
        /// What was asked.
        operation: GitOperation,
        /// Why the command did not run to its end.
        source: CaptureError,
    },
    /// Git refused: it ended with a failure.
    #[error("git {operation} failed with {exit:?}: {stderr}")]
    Failed {
        /// What was asked.
        operation: GitOperation,
        /// How git ended.
        exit: ExitReport,
        /// What git said, cut; invalid UTF-8 in it is replaced, because it is for a human to read.
        stderr: String,
    },
    /// The revision names no commit.
    #[error("the revision {revision:?} names no commit")]
    UnknownRevision {
        /// The revision as git was asked.
        revision: String,
    },
    /// The answer is longer than the bytes that are kept, so it cannot be read whole.
    #[error("the output of git {operation} is longer than {limit} bytes")]
    OutputTooLong {
        /// What was asked.
        operation: GitOperation,
        /// The bytes kept.
        limit: u64,
    },
    /// The answer is not UTF-8 text.
    #[error("the output of git {operation} is not UTF-8 text")]
    NotUtf8 {
        /// What was asked.
        operation: GitOperation,
        /// Where the bytes stop being UTF-8.
        source: FromUtf8Error,
    },
    /// The answer is not what git prints for this operation.
    #[error("git {operation} printed something unexpected: {output:?}")]
    Unexpected {
        /// What was asked.
        operation: GitOperation,
        /// What it printed, cut.
        output: String,
    },
    /// What git printed for a revision is not a commit ID.
    #[error("git {operation} did not print a commit ID")]
    NotACommit {
        /// What was asked.
        operation: GitOperation,
        /// Why the text is not a commit ID.
        source: InvalidCommitId,
    },
    /// The message of a merge is empty.
    #[error("the message of a merge must not be empty")]
    EmptyMessage,
    /// A merge is already under way: it must be finished or aborted first.
    #[error("a merge is already in progress; resolve its conflicts or abort it first")]
    MergeInProgress,
}

/// The branch a new worktree has out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeBranch {
    /// A new branch, made from `start`.
    New {
        /// The branch to create.
        branch: BranchName,
        /// Where it starts.
        start: Revision,
    },
    /// A branch that exists and is not out anywhere else.
    Existing(BranchName),
}

/// Whether a worktree is removed even when it has changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveMode {
    /// Only a worktree without changes.
    Clean,
    /// Whatever is in it.
    Force,
}

/// Whether a branch is deleted even when it has commits no other branch has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMode {
    /// Only a branch that is merged.
    Merged,
    /// Whatever is on it.
    Force,
}

/// How a merge ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// A merge commit was made; this is the new `HEAD`.
    Merged {
        /// The merge commit.
        head: CommitId,
    },
    /// The branch had nothing the checkout does not have.
    AlreadyUpToDate,
    /// The changes collide; the merge waits in the tree for the conflicts to be resolved or for an abort.
    Conflicted,
}

/// Whether a merge is under way in a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeState {
    /// There is a merge waiting for its conflicts.
    InProgress,
    /// There is none.
    NotMerging,
}

/// One changed path in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// What the index says of the path (`M`, `A`, `D`, `R`, `?`, ...), as `git status --porcelain` writes it.
    pub index: char,
    /// What the working tree says of the path, in the same letters.
    pub worktree: char,
    /// The path, relative to the root of the checkout; for a rename, the new one.
    pub path: PathBuf,
}

/// Whether a checkout has changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeState {
    /// Nothing is changed or untracked.
    Clean,
    /// These paths are changed or untracked.
    Dirty(Vec<StatusEntry>),
}

/// Git, run in one checkout with one identity.
#[derive(Debug, Clone)]
pub struct Git {
    /// The checkout (a repository or one of its worktrees) the commands run in.
    dir: AbsolutePath,
    /// The identity of the commits.
    identity: CommitIdentity,
    /// Variables added to the environment of every command.
    env: BTreeMap<EnvName, String>,
    /// The bounds of a command.
    limits: GitLimits,
}

impl Git {
    /// Git in `dir`, committing as `identity`.
    #[must_use]
    pub fn new(dir: AbsolutePath, identity: CommitIdentity) -> Self {
        Self {
            dir,
            identity,
            env: BTreeMap::new(),
            limits: GitLimits::default(),
        }
    }

    /// The same, with `name` set to `value` in the environment of every command: a credential helper's variables, or
    /// a git configuration a test pins.
    #[must_use]
    pub fn with_env(mut self, name: EnvName, value: impl Into<String>) -> Self {
        self.env.insert(name, value.into());
        self
    }

    /// The same, with other bounds.
    #[must_use]
    pub fn with_limits(mut self, limits: GitLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The commit `revision` names.
    ///
    /// # Errors
    /// [`GitError::UnknownRevision`] when it names no commit; [`GitError::Run`], [`GitError::Failed`],
    /// [`GitError::OutputTooLong`], [`GitError::NotUtf8`] and [`GitError::Unexpected`].
    pub async fn rev_parse(&self, revision: &Revision) -> Result<CommitId, GitError> {
        let operation = GitOperation::RevParse;
        let spec = format!("{revision}^{{commit}}");
        let captured = self
            .run(
                operation,
                Reach::Local,
                &[
                    arg("rev-parse"),
                    arg("--verify"),
                    arg("--quiet"),
                    arg("--end-of-options"),
                    arg(&spec),
                ],
            )
            .await?;
        match captured.exit {
            ExitReport::Exited { code: 0 } => {}
            // With `--verify --quiet`, 1 is git's answer for a revision that names nothing.
            ExitReport::Exited { code: 1 } => {
                return Err(GitError::UnknownRevision {
                    revision: revision.to_string(),
                });
            }
            ExitReport::Exited { .. } | ExitReport::Signaled { .. } => return Err(failed(operation, &captured)),
        }
        let printed = text(operation, &captured.stdout)?;
        printed
            .trim_end()
            .parse()
            .map_err(|source| GitError::NotACommit { operation, source })
    }

    /// The tip of `branch`, or `None` when there is no such branch.
    ///
    /// # Errors
    /// The errors of [`Git::rev_parse`] other than an unknown revision.
    pub async fn branch_tip(&self, branch: &BranchName) -> Result<Option<CommitId>, GitError> {
        match self.rev_parse(&Revision::Branch(branch.clone())).await {
            Ok(tip) => Ok(Some(tip)),
            // A branch that does not exist is the answer, not a failure.
            Err(GitError::UnknownRevision { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Makes `branch` point at `start`.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the branch exists or `start` is unknown; [`GitError::Run`].
    pub async fn branch_create(&self, branch: &BranchName, start: &Revision) -> Result<(), GitError> {
        let start = start.to_string();
        self.run_ok(
            GitOperation::BranchCreate,
            Reach::Local,
            &[arg("branch"), arg(branch.as_str()), arg(&start)],
        )
        .await
        .map(drop)
    }

    /// Deletes `branch`.
    ///
    /// # Errors
    /// [`GitError::Failed`] when it does not exist, is out in a worktree, or (with [`DeleteMode::Merged`]) is not
    /// merged; [`GitError::Run`].
    pub async fn branch_delete(&self, branch: &BranchName, mode: DeleteMode) -> Result<(), GitError> {
        let flag = match mode {
            DeleteMode::Merged => "-d",
            DeleteMode::Force => "-D",
        };
        self.run_ok(
            GitOperation::BranchDelete,
            Reach::Local,
            &[arg("branch"), arg(flag), arg(branch.as_str())],
        )
        .await
        .map(drop)
    }

    /// Adds a worktree at `path` with `branch` out.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the path is taken, the branch exists (for a new one) or is out elsewhere (for an
    /// existing one); [`GitError::Run`].
    pub async fn worktree_add(&self, path: &AbsolutePath, branch: &WorktreeBranch) -> Result<(), GitError> {
        let path = path.as_path().as_os_str();
        let captured = match branch {
            WorktreeBranch::New { branch, start } => {
                let start = start.to_string();
                self.run_ok(
                    GitOperation::WorktreeAdd,
                    Reach::Local,
                    &[
                        arg("worktree"),
                        arg("add"),
                        arg("-b"),
                        arg(branch.as_str()),
                        path,
                        arg(&start),
                    ],
                )
                .await?
            }
            WorktreeBranch::Existing(branch) => {
                self.run_ok(
                    GitOperation::WorktreeAdd,
                    Reach::Local,
                    &[arg("worktree"), arg("add"), path, arg(branch.as_str())],
                )
                .await?
            }
        };
        drop(captured);
        Ok(())
    }

    /// Removes the worktree at `path`.
    ///
    /// # Errors
    /// [`GitError::Failed`] when it is not a worktree, or has changes and `mode` is [`RemoveMode::Clean`];
    /// [`GitError::Run`].
    pub async fn worktree_remove(&self, path: &AbsolutePath, mode: RemoveMode) -> Result<(), GitError> {
        let path = path.as_path().as_os_str();
        let args: &[&OsStr] = match mode {
            RemoveMode::Clean => &[arg("worktree"), arg("remove"), path],
            RemoveMode::Force => &[arg("worktree"), arg("remove"), arg("--force"), path],
        };
        self.run_ok(GitOperation::WorktreeRemove, Reach::Local, args)
            .await
            .map(drop)
    }

    /// Whether the checkout has changes, and which.
    ///
    /// # Errors
    /// [`GitError::Run`], [`GitError::Failed`], [`GitError::OutputTooLong`] and [`GitError::Unexpected`].
    pub async fn tree_state(&self) -> Result<TreeState, GitError> {
        let operation = GitOperation::Status;
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("--no-optional-locks"),
                    arg("status"),
                    arg("--porcelain=v1"),
                    arg("-z"),
                    arg("--untracked-files=all"),
                ],
            )
            .await?;
        let entries = parse_status(operation, whole(operation, &captured.stdout, self.limits)?)?;
        Ok(if entries.is_empty() {
            TreeState::Clean
        } else {
            TreeState::Dirty(entries)
        })
    }

    /// Whether a merge is under way.
    ///
    /// # Errors
    /// [`GitError::Run`] and [`GitError::Failed`].
    pub async fn merge_state(&self) -> Result<MergeState, GitError> {
        let operation = GitOperation::MergeState;
        let captured = self
            .run(
                operation,
                Reach::Local,
                &[arg("rev-parse"), arg("-q"), arg("--verify"), arg("MERGE_HEAD")],
            )
            .await?;
        match captured.exit {
            ExitReport::Exited { code: 0 } => Ok(MergeState::InProgress),
            // With `-q --verify`, 1 is git's answer for a reference that does not exist.
            ExitReport::Exited { code: 1 } => Ok(MergeState::NotMerging),
            ExitReport::Exited { .. } | ExitReport::Signaled { .. } => Err(failed(operation, &captured)),
        }
    }

    /// Merges `branch` into what the checkout has out, always with a merge commit (no fast-forward), made with
    /// `message` and the identity of this handle.
    ///
    /// A merge that conflicts is an answer, [`MergeOutcome::Conflicted`]: it stays in the tree.
    ///
    /// # Errors
    /// [`GitError::EmptyMessage`]; [`GitError::MergeInProgress`] while another merge waits for its conflicts;
    /// [`GitError::Failed`] when git refuses without a conflict (an unknown branch, a
    /// dirty tree that the merge would overwrite); [`GitError::Run`] and the errors of [`Git::rev_parse`].
    pub async fn merge_no_ff(&self, branch: &BranchName, message: &str) -> Result<MergeOutcome, GitError> {
        if message.trim().is_empty() {
            return Err(GitError::EmptyMessage);
        }
        // Git refuses a merge while one waits, and `MERGE_HEAD` is then still there: ask first, or the refusal
        // below would be taken for a conflict of this merge.
        if self.merge_state().await? == MergeState::InProgress {
            return Err(GitError::MergeInProgress);
        }
        let before = self.rev_parse(&Revision::Head).await?;
        let target = Revision::Branch(branch.clone()).to_string();
        let captured = self
            .run(
                GitOperation::Merge,
                Reach::Local,
                &[
                    arg("merge"),
                    arg("--no-ff"),
                    arg("--no-edit"),
                    arg("-m"),
                    arg(message),
                    arg(&target),
                ],
            )
            .await?;
        if captured.exit.succeeded() {
            let after = self.rev_parse(&Revision::Head).await?;
            return Ok(if after == before {
                MergeOutcome::AlreadyUpToDate
            } else {
                MergeOutcome::Merged { head: after }
            });
        }
        match self.merge_state().await? {
            MergeState::InProgress => Ok(MergeOutcome::Conflicted),
            MergeState::NotMerging => Err(failed(GitOperation::Merge, &captured)),
        }
    }

    /// Abandons the merge that is under way and restores the tree.
    ///
    /// # Errors
    /// [`GitError::Failed`] when no merge is under way; [`GitError::Run`].
    pub async fn merge_abort(&self) -> Result<(), GitError> {
        self.run_ok(GitOperation::MergeAbort, Reach::Local, &[arg("merge"), arg("--abort")])
            .await
            .map(drop)
    }

    /// Pushes `branch` to `origin` and makes it the branch's upstream.
    ///
    /// # Errors
    /// [`GitError::Failed`] when there is no `origin`, the remote refuses (a branch that is not a fast-forward), or
    /// the credentials fail; [`GitError::Run`], for example when the command ran over its time.
    pub async fn push(&self, branch: &BranchName) -> Result<(), GitError> {
        let reference = format!("refs/heads/{branch}:refs/heads/{branch}");
        self.run_ok(
            GitOperation::Push,
            Reach::Network,
            &[arg("push"), arg("--set-upstream"), arg(REMOTE), arg(&reference)],
        )
        .await
        .map(drop)
    }

    /// The command `git` with this handle's identity, signing and environment, and `args`.
    fn command(&self, args: &[&OsStr]) -> Command {
        let mut command = Command::new("git");
        command.current_dir(self.dir.as_path());
        match &self.identity.signing_key {
            Some(key) => {
                let mut signing_key = OsString::from("user.signingkey=");
                signing_key.push(key.as_path());
                command
                    .args(["-c", "commit.gpgsign=true", "-c", "gpg.format=ssh", "-c"])
                    .arg(signing_key);
            }
            // The human's own configuration may sign every commit with a key the harness does not have.
            None => {
                command.args(["-c", "commit.gpgsign=false"]);
            }
        }
        command
            .args(args)
            .env("GIT_AUTHOR_NAME", &self.identity.name)
            .env("GIT_AUTHOR_EMAIL", &self.identity.email)
            .env("GIT_COMMITTER_NAME", &self.identity.name)
            .env("GIT_COMMITTER_EMAIL", &self.identity.email)
            // Nobody answers a prompt for credentials; a missing credential must fail, not wait.
            .env("GIT_TERMINAL_PROMPT", "0")
            // Messages that are matched or shown are in one language.
            .env("LC_ALL", "C");
        for (name, value) in &self.env {
            command.env(name.as_str(), value);
        }
        command
    }

    /// Runs git with `args` and returns what it did, whether it succeeded or not.
    async fn run(&self, operation: GitOperation, reach: Reach, args: &[&OsStr]) -> Result<Captured, GitError> {
        let timeout = match reach {
            Reach::Local => self.limits.local_timeout,
            Reach::Network => self.limits.network_timeout,
        };
        let limits = CaptureLimits {
            timeout,
            max_bytes: self.limits.max_output_bytes,
        };
        capture(self.command(args), limits)
            .await
            .map_err(|source| GitError::Run { operation, source })
    }

    /// Runs git with `args`; a command that ends with a failure is an error.
    async fn run_ok(&self, operation: GitOperation, reach: Reach, args: &[&OsStr]) -> Result<Captured, GitError> {
        let captured = self.run(operation, reach, args).await?;
        if captured.exit.succeeded() {
            Ok(captured)
        } else {
            Err(failed(operation, &captured))
        }
    }
}

/// Where a command goes, which decides its time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The local repository only.
    Local,
    /// A remote.
    Network,
}

/// `text` as an argument.
fn arg(text: &str) -> &OsStr {
    OsStr::new(text)
}

/// A few characters of what git (or a file) printed, for an error message: invalid UTF-8 is replaced, because the
/// text is for a human to read.
fn diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim()
        .chars()
        .take(MAX_DIAGNOSTIC_CHARS)
        .collect()
}

/// The error for a command that ended with a failure.
fn failed(operation: GitOperation, captured: &Captured) -> GitError {
    GitError::Failed {
        operation,
        exit: captured.exit,
        stderr: diagnostic(&captured.stderr.bytes),
    }
}

/// The bytes of `stream`, when it is whole.
fn whole(operation: GitOperation, stream: &Stream, limits: GitLimits) -> Result<&[u8], GitError> {
    if stream.dropped > 0 {
        return Err(GitError::OutputTooLong {
            operation,
            limit: limits.max_output_bytes,
        });
    }
    Ok(&stream.bytes)
}

/// The text of `stream`.
fn text(operation: GitOperation, stream: &Stream) -> Result<String, GitError> {
    String::from_utf8(stream.bytes.clone()).map_err(|source| GitError::NotUtf8 { operation, source })
}

/// Reads the output of `git status --porcelain=v1 -z`: `XY path` entries, a renamed or copied path followed by the
/// path it came from.
fn parse_status(operation: GitOperation, bytes: &[u8]) -> Result<Vec<StatusEntry>, GitError> {
    let mut entries = Vec::new();
    let mut tokens = bytes.split(|byte| *byte == 0).filter(|token| !token.is_empty());
    while let Some(token) = tokens.next() {
        let unexpected = || GitError::Unexpected {
            operation,
            output: diagnostic(token),
        };
        let Some(&[index, worktree, b' ']) = token.first_chunk::<3>() else {
            return Err(unexpected());
        };
        let path = token.get(3..).filter(|path| !path.is_empty()).ok_or_else(unexpected)?;
        if matches!(index, b'R' | b'C') || matches!(worktree, b'R' | b'C') {
            tokens.next().ok_or_else(unexpected)?;
        }
        entries.push(StatusEntry {
            index: char::from(index),
            worktree: char::from(worktree),
            path: PathBuf::from(OsStr::from_bytes(path)),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;

    use super::*;

    /// A git that no configuration of the machine reaches.
    fn isolated(dir: &Path, identity: CommitIdentity) -> Git {
        Git::new(abs(dir), identity)
            .with_env(env("GIT_CONFIG_GLOBAL"), "/dev/null")
            .with_env(env("GIT_CONFIG_NOSYSTEM"), "1")
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn env(name: &str) -> EnvName {
        name.parse().unwrap()
    }

    fn branch(name: &str) -> BranchName {
        name.parse().unwrap()
    }

    fn agent_identity() -> CommitIdentity {
        CommitIdentity::new(
            "codex-1 (team-agents)".to_owned(),
            "codex-1@team-agents.invalid".to_owned(),
            None,
        )
    }

    /// Runs real git for the setup of a test, as someone else than the agent, and returns its trimmed output.
    fn raw(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "protocol.file.allow=always"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
    }

    /// A repository with one commit on `main`, in a temporary directory.
    struct Fixture {
        /// Keeps the directory alive.
        _tmp: tempfile::TempDir,
        /// The repository.
        repo: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let repo = tmp.path().canonicalize().unwrap().join("repo");
            std::fs::create_dir(&repo).unwrap();
            raw(&repo, &["init", "-q", "-b", "main"]);
            let fixture = Self { _tmp: tmp, repo };
            fixture.commit("a.txt", "one\n", "First");
            fixture
        }

        /// Commits `content` as `name` in the repository's checkout.
        fn commit(&self, name: &str, content: &str, message: &str) {
            commit_in(&self.repo, name, content, message);
        }

        fn git(&self) -> Git {
            isolated(&self.repo, agent_identity())
        }

        fn head(&self) -> CommitId {
            raw(&self.repo, &["rev-parse", "HEAD"]).parse().unwrap()
        }

        /// A directory beside the repository, for worktrees and remotes.
        fn sibling(&self, name: &str) -> PathBuf {
            self.repo.parent().unwrap().join(name)
        }
    }

    fn commit_in(dir: &Path, name: &str, content: &str, message: &str) {
        std::fs::write(dir.join(name), content).unwrap();
        raw(dir, &["add", name]);
        raw(dir, &["commit", "-q", "-m", message]);
    }

    /// Makes `topic` from `main` with one commit that adds `topic.txt`, and goes back to `main`.
    fn topic_branch(fixture: &Fixture) {
        raw(&fixture.repo, &["checkout", "-q", "-b", "topic"]);
        fixture.commit("topic.txt", "topic\n", "Topic work");
        raw(&fixture.repo, &["checkout", "-q", "main"]);
    }

    // ---- revisions and branches ---------------------------------------------------------------------------------

    // Rule: HEAD, a branch and a commit name the same commit when they point at it.
    #[tokio::test]
    async fn rev_parse_names_the_commit_of_head_a_branch_and_a_commit() {
        let fixture = Fixture::new();
        let head = fixture.head();

        let by_head = fixture.git().rev_parse(&Revision::Head).await.unwrap();
        let by_branch = fixture
            .git()
            .rev_parse(&Revision::Branch(branch("main")))
            .await
            .unwrap();
        let by_commit = fixture.git().rev_parse(&Revision::Commit(head.clone())).await.unwrap();

        assert_eq!((by_head, by_branch, by_commit), (head.clone(), head.clone(), head));
    }

    // Rule: a revision that names nothing is its own error.
    #[tokio::test]
    async fn rev_parse_of_an_unknown_revision_says_which() {
        let fixture = Fixture::new();

        let result = fixture.git().rev_parse(&Revision::Branch(branch("nothing"))).await;

        assert!(
            matches!(&result, Err(GitError::UnknownRevision { revision }) if revision == "refs/heads/nothing"),
            "{result:?}"
        );
    }

    // Rule: a branch is made at a start, found by its tip, and deleted.
    #[tokio::test]
    async fn a_branch_is_created_found_and_deleted() {
        let fixture = Fixture::new();
        let git = fixture.git();
        let topic = branch("agents/codex-1/task");
        assert_eq!(git.branch_tip(&topic).await.unwrap(), None);

        git.branch_create(&topic, &Revision::Head).await.unwrap();
        assert_eq!(git.branch_tip(&topic).await.unwrap(), Some(fixture.head()));

        git.branch_delete(&topic, DeleteMode::Merged).await.unwrap();
        assert_eq!(git.branch_tip(&topic).await.unwrap(), None);
    }

    // Rule: a branch that exists cannot be created again.
    #[tokio::test]
    async fn creating_a_branch_that_exists_fails() {
        let fixture = Fixture::new();

        let result = fixture.git().branch_create(&branch("main"), &Revision::Head).await;

        assert!(
            matches!(
                &result,
                Err(GitError::Failed {
                    operation: GitOperation::BranchCreate,
                    exit: ExitReport::Exited { code: 128 },
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a branch with commits of its own needs the force to be deleted.
    #[tokio::test]
    async fn an_unmerged_branch_is_deleted_only_by_force() {
        let fixture = Fixture::new();
        topic_branch(&fixture);
        let git = fixture.git();

        let refused = git.branch_delete(&branch("topic"), DeleteMode::Merged).await;
        assert!(
            matches!(
                &refused,
                Err(GitError::Failed {
                    operation: GitOperation::BranchDelete,
                    ..
                })
            ),
            "{refused:?}"
        );

        git.branch_delete(&branch("topic"), DeleteMode::Force).await.unwrap();
        assert_eq!(git.branch_tip(&branch("topic")).await.unwrap(), None);
    }

    // ---- worktrees ----------------------------------------------------------------------------------------------

    // Rule: a worktree with a new branch has the files of the start and the branch at it.
    #[tokio::test]
    async fn a_worktree_gets_a_new_branch_at_its_start() {
        let fixture = Fixture::new();
        let path = fixture.sibling("work");
        let task = branch("agents/codex-1/task-1");

        fixture
            .git()
            .worktree_add(
                &abs(&path),
                &WorktreeBranch::New {
                    branch: task.clone(),
                    start: Revision::Branch(branch("main")),
                },
            )
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(path.join("a.txt")).unwrap(), "one\n");
        assert_eq!(
            raw(&path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "agents/codex-1/task-1"
        );
        assert_eq!(fixture.git().branch_tip(&task).await.unwrap(), Some(fixture.head()));
    }

    // Rule: a worktree can take a branch that exists.
    #[tokio::test]
    async fn a_worktree_can_take_an_existing_branch() {
        let fixture = Fixture::new();
        topic_branch(&fixture);
        let path = fixture.sibling("work");

        fixture
            .git()
            .worktree_add(&abs(&path), &WorktreeBranch::Existing(branch("topic")))
            .await
            .unwrap();

        assert!(path.join("topic.txt").is_file());
    }

    // Rule: a branch that is out in another worktree cannot be taken, and the error is git's.
    #[tokio::test]
    async fn a_branch_that_is_out_elsewhere_cannot_be_taken_by_a_worktree() {
        let fixture = Fixture::new();

        let result = fixture
            .git()
            .worktree_add(
                &abs(&fixture.sibling("work")),
                &WorktreeBranch::Existing(branch("main")),
            )
            .await;

        assert!(
            matches!(
                &result,
                Err(GitError::Failed {
                    operation: GitOperation::WorktreeAdd,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a worktree without changes is removed; one with changes only by force.
    #[tokio::test]
    async fn a_worktree_with_changes_is_removed_only_by_force() {
        let fixture = Fixture::new();
        let path = fixture.sibling("work");
        let git = fixture.git();
        git.worktree_add(
            &abs(&path),
            &WorktreeBranch::New {
                branch: branch("task-1"),
                start: Revision::Head,
            },
        )
        .await
        .unwrap();
        std::fs::write(path.join("scratch.txt"), "x").unwrap();

        let refused = git.worktree_remove(&abs(&path), RemoveMode::Clean).await;
        assert!(
            matches!(
                &refused,
                Err(GitError::Failed {
                    operation: GitOperation::WorktreeRemove,
                    ..
                })
            ),
            "{refused:?}"
        );
        assert!(path.exists());

        git.worktree_remove(&abs(&path), RemoveMode::Force).await.unwrap();
        assert!(!path.exists());
    }

    // Rule: a clean worktree is removed.
    #[tokio::test]
    async fn a_clean_worktree_is_removed() {
        let fixture = Fixture::new();
        let path = fixture.sibling("work");
        let git = fixture.git();
        git.worktree_add(
            &abs(&path),
            &WorktreeBranch::New {
                branch: branch("task-1"),
                start: Revision::Head,
            },
        )
        .await
        .unwrap();

        git.worktree_remove(&abs(&path), RemoveMode::Clean).await.unwrap();

        assert!(!path.exists());
    }

    // ---- the state of the tree ----------------------------------------------------------------------------------

    // Rule: a checkout without changes is clean.
    #[tokio::test]
    async fn a_checkout_without_changes_is_clean() {
        let fixture = Fixture::new();

        assert_eq!(fixture.git().tree_state().await.unwrap(), TreeState::Clean);
    }

    // Rule: changed, staged, deleted, renamed and untracked paths are listed, with names git would quote.
    #[tokio::test]
    async fn a_dirty_checkout_lists_its_paths() {
        let fixture = Fixture::new();
        fixture.commit("b.txt", "two\n", "Second");
        fixture.commit("old name.txt", "three\n", "Third");
        std::fs::write(fixture.repo.join("a.txt"), "changed\n").unwrap();
        raw(&fixture.repo, &["mv", "old name.txt", "new żółć.txt"]);
        std::fs::remove_file(fixture.repo.join("b.txt")).unwrap();
        std::fs::write(fixture.repo.join("untracked.txt"), "x").unwrap();

        let state = fixture.git().tree_state().await.unwrap();

        let TreeState::Dirty(mut entries) = state else {
            panic!("the checkout is dirty: {state:?}")
        };
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let entry = |index, worktree, path: &str| StatusEntry {
            index,
            worktree,
            path: PathBuf::from(path),
        };
        assert_eq!(
            entries,
            vec![
                entry(' ', 'M', "a.txt"),
                entry(' ', 'D', "b.txt"),
                entry('R', ' ', "new żółć.txt"),
                entry('?', '?', "untracked.txt"),
            ]
        );
    }

    // Rule: an answer longer than the bytes that are kept is an error, not a cut list.
    #[tokio::test]
    async fn a_status_longer_than_the_limit_is_an_error() {
        let fixture = Fixture::new();
        std::fs::write(fixture.repo.join("a-file-with-a-rather-long-name.txt"), "x").unwrap();
        let git = fixture.git().with_limits(GitLimits {
            max_output_bytes: 8,
            ..GitLimits::default()
        });

        let result = git.tree_state().await;

        assert!(
            matches!(
                &result,
                Err(GitError::OutputTooLong {
                    operation: GitOperation::Status,
                    limit: 8
                })
            ),
            "{result:?}"
        );
    }

    // ---- merges -------------------------------------------------------------------------------------------------

    // Rule: a merge makes a merge commit with two parents even when a fast-forward is possible; the commit has the
    // agent's identity and the message, and the files of the branch.
    #[tokio::test]
    async fn merge_makes_a_merge_commit_by_the_agent() {
        let fixture = Fixture::new();
        topic_branch(&fixture);
        let before = fixture.head();

        let outcome = fixture
            .git()
            .merge_no_ff(&branch("topic"), "Merge the topic")
            .await
            .unwrap();

        let head = fixture.head();
        assert_eq!(outcome, MergeOutcome::Merged { head: head.clone() });
        assert_eq!(raw(&fixture.repo, &["rev-parse", "HEAD^1"]), before.to_string());
        assert_eq!(
            raw(&fixture.repo, &["rev-parse", "HEAD^2"]),
            raw(&fixture.repo, &["rev-parse", "topic"])
        );
        assert_eq!(
            raw(&fixture.repo, &["log", "-1", "--format=%an <%ae>|%cn <%ce>|%s"]),
            "codex-1 (team-agents) <codex-1@team-agents.invalid>|codex-1 (team-agents) <codex-1@team-agents.invalid>|Merge the topic"
        );
        assert!(fixture.repo.join("topic.txt").is_file());
    }

    // Rule: a branch with nothing new is an answer, not a commit.
    #[tokio::test]
    async fn merging_a_branch_with_nothing_new_changes_nothing() {
        let fixture = Fixture::new();
        raw(&fixture.repo, &["branch", "same"]);
        let before = fixture.head();

        let outcome = fixture.git().merge_no_ff(&branch("same"), "Merge").await.unwrap();

        assert_eq!(outcome, MergeOutcome::AlreadyUpToDate);
        assert_eq!(fixture.head(), before);
    }

    // Rule: a conflict is an answer; the merge waits in the tree until it is aborted, and abort restores the tree.
    #[tokio::test]
    async fn a_conflicting_merge_waits_and_can_be_aborted() {
        let fixture = Fixture::new();
        raw(&fixture.repo, &["checkout", "-q", "-b", "left"]);
        fixture.commit("a.txt", "left\n", "Left");
        raw(&fixture.repo, &["checkout", "-q", "main"]);
        fixture.commit("a.txt", "main\n", "Main");
        let git = fixture.git();
        assert_eq!(git.merge_state().await.unwrap(), MergeState::NotMerging);

        let outcome = git.merge_no_ff(&branch("left"), "Merge left").await.unwrap();

        assert_eq!(outcome, MergeOutcome::Conflicted);
        assert_eq!(git.merge_state().await.unwrap(), MergeState::InProgress);
        assert!(matches!(git.tree_state().await.unwrap(), TreeState::Dirty(_)));

        git.merge_abort().await.unwrap();

        assert_eq!(git.merge_state().await.unwrap(), MergeState::NotMerging);
        assert_eq!(git.tree_state().await.unwrap(), TreeState::Clean);
    }

    // Rule: a second merge while one waits is refused — it must not be taken for a new conflict.
    #[tokio::test]
    async fn a_merge_while_another_waits_is_refused() {
        let fixture = Fixture::new();
        raw(&fixture.repo, &["checkout", "-q", "-b", "left"]);
        fixture.commit("a.txt", "left\n", "Left");
        raw(&fixture.repo, &["checkout", "-q", "main"]);
        fixture.commit("a.txt", "main\n", "Main");
        topic_branch(&fixture);
        let git = fixture.git();
        assert_eq!(
            git.merge_no_ff(&branch("left"), "Merge left").await.unwrap(),
            MergeOutcome::Conflicted
        );

        let result = git.merge_no_ff(&branch("topic"), "Merge topic").await;

        assert!(matches!(&result, Err(GitError::MergeInProgress)), "{result:?}");
        assert_eq!(git.merge_state().await.unwrap(), MergeState::InProgress);
    }

    // Rule: a merge that git refuses for another reason than a conflict is an error with git's words.
    #[tokio::test]
    async fn merging_an_unknown_branch_fails() {
        let fixture = Fixture::new();

        let result = fixture.git().merge_no_ff(&branch("nothing"), "Merge").await;

        assert!(
            matches!(&result, Err(GitError::Failed { operation: GitOperation::Merge, stderr, .. }) if stderr.contains("nothing")),
            "{result:?}"
        );
    }

    // Rule: aborting when no merge is under way is git's error.
    #[tokio::test]
    async fn aborting_without_a_merge_fails() {
        let fixture = Fixture::new();

        let result = fixture.git().merge_abort().await;

        assert!(
            matches!(
                &result,
                Err(GitError::Failed {
                    operation: GitOperation::MergeAbort,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a merge needs a message.
    #[tokio::test]
    async fn a_merge_without_a_message_is_refused() {
        let fixture = Fixture::new();
        topic_branch(&fixture);

        let result = fixture.git().merge_no_ff(&branch("topic"), "  \n").await;

        assert!(matches!(&result, Err(GitError::EmptyMessage)), "{result:?}");
        assert_eq!(raw(&fixture.repo, &["rev-list", "--count", "HEAD"]), "1");
    }

    // ---- identity and signing -----------------------------------------------------------------------------------

    /// A global configuration that signs everything with a key that does not exist, as a human's might.
    fn signing_everything_config(dir: &Path) -> PathBuf {
        let path = dir.join("global.gitconfig");
        std::fs::write(
            &path,
            "[commit]\n\tgpgsign = true\n[gpg]\n\tformat = ssh\n[user]\n\tsigningkey = /nonexistent/key\n",
        )
        .unwrap();
        path
    }

    // Rule: without a signing key the commits are unsigned, even if the human's own git signs everything.
    #[tokio::test]
    async fn without_a_key_the_commit_is_unsigned_whatever_the_global_configuration_says() {
        let fixture = Fixture::new();
        topic_branch(&fixture);
        let config = signing_everything_config(fixture.repo.parent().unwrap());
        let git = fixture
            .git()
            .with_env(env("GIT_CONFIG_GLOBAL"), config.to_str().unwrap());

        let outcome = git.merge_no_ff(&branch("topic"), "Merge").await.unwrap();

        assert!(matches!(outcome, MergeOutcome::Merged { .. }));
        let commit = raw(&fixture.repo, &["cat-file", "commit", "HEAD"]);
        assert!(!commit.contains("gpgsig"), "{commit}");
    }

    // Rule: with a signing key the commit is signed with it.
    #[tokio::test]
    async fn with_a_key_the_commit_is_signed() {
        let fixture = Fixture::new();
        topic_branch(&fixture);
        let key = fixture.sibling("signing-key");
        let generated = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
            .arg(&key)
            .output()
            .unwrap();
        assert!(generated.status.success(), "{generated:?}");
        let identity = CommitIdentity::new("codex-1".to_owned(), "codex-1@example.com".to_owned(), Some(abs(&key)));

        let outcome = isolated(&fixture.repo, identity)
            .merge_no_ff(&branch("topic"), "Merge")
            .await
            .unwrap();

        assert!(matches!(outcome, MergeOutcome::Merged { .. }));
        let commit = raw(&fixture.repo, &["cat-file", "commit", "HEAD"]);
        assert!(commit.contains("gpgsig -----BEGIN SSH SIGNATURE-----"), "{commit}");
    }

    // Rule: the identity of an agent is rendered from the configuration's templates.
    #[test]
    fn the_identity_of_an_agent_comes_from_the_configuration() {
        let config = GitIdentity::default();
        let agent: AgentName = "claude-2".parse().unwrap();

        let identity = CommitIdentity::for_agent(&config, &agent);

        assert_eq!(
            identity,
            CommitIdentity::new(
                "claude-2 (team-agents)".to_owned(),
                "claude-2@team-agents.invalid".to_owned(),
                None
            )
        );
    }

    // ---- push ---------------------------------------------------------------------------------------------------

    /// A bare repository as the `origin` of `fixture`.
    fn with_origin(fixture: &Fixture) -> PathBuf {
        let origin = fixture.sibling("origin.git");
        std::fs::create_dir(&origin).unwrap();
        raw(&origin, &["init", "-q", "--bare", "-b", "main"]);
        raw(&fixture.repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        origin
    }

    // Rule: a branch is pushed to origin and becomes the branch's upstream.
    #[tokio::test]
    async fn push_sends_the_branch_to_origin_and_sets_its_upstream() {
        let fixture = Fixture::new();
        let origin = with_origin(&fixture);
        topic_branch(&fixture);

        fixture.git().push(&branch("topic")).await.unwrap();

        assert_eq!(
            raw(&origin, &["rev-parse", "refs/heads/topic"]),
            raw(&fixture.repo, &["rev-parse", "topic"])
        );
        assert_eq!(raw(&fixture.repo, &["config", "branch.topic.remote"]), "origin");
    }

    // Rule: a push that is not a fast-forward is refused, never forced.
    #[tokio::test]
    async fn a_push_that_is_not_a_fast_forward_fails() {
        let fixture = Fixture::new();
        let origin = with_origin(&fixture);
        let git = fixture.git();
        git.push(&branch("main")).await.unwrap();
        raw(&fixture.repo, &["reset", "-q", "--hard", "HEAD"]);
        let other = fixture.sibling("other");
        raw(
            fixture.repo.parent().unwrap(),
            &["clone", "-q", origin.to_str().unwrap(), other.to_str().unwrap()],
        );
        commit_in(&other, "elsewhere.txt", "x", "Elsewhere");
        raw(&other, &["push", "-q", "origin", "main"]);
        fixture.commit("here.txt", "y", "Here");

        let result = git.push(&branch("main")).await;

        assert!(
            matches!(
                &result,
                Err(GitError::Failed {
                    operation: GitOperation::Push,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a repository without origin cannot be pushed to.
    #[tokio::test]
    async fn pushing_without_an_origin_fails() {
        let fixture = Fixture::new();

        let result = fixture.git().push(&branch("main")).await;

        assert!(
            matches!(
                &result,
                Err(GitError::Failed {
                    operation: GitOperation::Push,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // ---- running ------------------------------------------------------------------------------------------------

    // Rule: a command that runs over its time is killed and reported, also when a hook hangs.
    #[tokio::test]
    async fn a_command_over_its_time_is_an_error() {
        let fixture = Fixture::new();
        topic_branch(&fixture);
        let hook = fixture.repo.join(".git/hooks/pre-merge-commit");
        std::fs::write(&hook, "#!/bin/sh\nsleep 300\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = fixture.git().with_limits(GitLimits {
            local_timeout: Duration::from_millis(500),
            ..GitLimits::default()
        });

        let result = git.merge_no_ff(&branch("topic"), "Merge").await;

        assert!(
            matches!(
                &result,
                Err(GitError::Run {
                    operation: GitOperation::Merge,
                    source: CaptureError::Timeout { .. }
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a checkout that does not exist is a failure to run git, not a git answer.
    #[tokio::test]
    async fn a_missing_checkout_is_a_run_error() {
        let tmp = tempfile::tempdir().unwrap();
        let git = isolated(&tmp.path().join("missing"), agent_identity());

        let result = git.rev_parse(&Revision::Head).await;

        assert!(
            matches!(
                &result,
                Err(GitError::Run {
                    operation: GitOperation::RevParse,
                    source: CaptureError::Spawn { .. }
                })
            ),
            "{result:?}"
        );
    }

    // ---- answers real git does not give -------------------------------------------------------------------------

    /// A git in a temporary directory whose `git` is a script that runs `body`: for the answers a real git never
    /// gives, which the code must still refuse with a typed error.
    fn scripted_git(body: &str) -> (tempfile::TempDir, Git) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let script = bin.join("git");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = Git::new(abs(dir.path()), agent_identity()).with_env(env("PATH"), bin.to_str().unwrap());
        (dir, git)
    }

    // Rule: what git prints for a revision must be a commit ID.
    #[tokio::test]
    async fn a_revision_that_prints_no_commit_id_is_an_error() {
        let (_dir, git) = scripted_git("printf 'not-a-commit\\n'");

        let result = git.rev_parse(&Revision::Head).await;

        assert!(
            matches!(
                &result,
                Err(GitError::NotACommit {
                    operation: GitOperation::RevParse,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: output that is not text is refused, not decoded by guessing.
    #[tokio::test]
    async fn output_that_is_not_utf8_is_an_error() {
        let (_dir, git) = scripted_git("printf '\\377\\376\\n'");

        let result = git.rev_parse(&Revision::Head).await;

        assert!(
            matches!(
                &result,
                Err(GitError::NotUtf8 {
                    operation: GitOperation::RevParse,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a status that is not the porcelain format is refused.
    #[tokio::test]
    async fn a_status_in_another_format_is_an_error() {
        let (_dir, git) = scripted_git("printf 'x\\0'");

        let result = git.tree_state().await;

        assert!(
            matches!(&result, Err(GitError::Unexpected { operation: GitOperation::Status, output }) if output == "x"),
            "{result:?}"
        );
    }

    // Rule: a rename that lacks the path it came from is refused.
    #[tokio::test]
    async fn a_rename_without_its_origin_is_an_error() {
        let (_dir, git) = scripted_git("printf 'R  new.txt\\0'");

        let result = git.tree_state().await;

        assert!(
            matches!(
                &result,
                Err(GitError::Unexpected {
                    operation: GitOperation::Status,
                    ..
                })
            ),
            "{result:?}"
        );
    }
}
