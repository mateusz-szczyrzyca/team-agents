//! What a run changed, as git says it: the commits between two heads with their author time, and the files the worktree
//! holds that no commit does. The history records it at the end of every run.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::EnvName;
use agent_core::decide::checks::FileChange;
use agent_core::history::CommitRecord;
use agent_core::state::{CommitId, Timestamp};

use super::range::{join_changes, parse_name_status, parse_numstat};
use super::{Git, GitError, GitOperation, Reach, Revision, ScratchError, arg, text, whole};

/// What a run changed in its worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkChanges {
    /// The commit the worktree is at now.
    pub head_after: CommitId,
    /// The commits between the head the run started at and the one it ended at, oldest first.
    pub commits: Vec<CommitRecord>,
    /// The files that differ between the two heads.
    pub files: Vec<FileChange>,
    /// The files that differ between the head and the tree: changed, new or gone, in no commit.
    pub uncommitted: Vec<FileChange>,
}

/// The fields of one commit of the log: its ID, the time it was authored (seconds since the epoch) and its subject.
const LOG_FIELDS: usize = 3;

impl Git {
    /// The commits after `base` up to `head`, oldest first, each with the first line of its message and when it was
    /// authored. A merge is listed after the commits it brought in; a bad byte of a subject is replaced.
    ///
    /// # Errors
    /// [`GitError::TooManyCommits`] when the range has more commits than [`GitLimits::max_commits`](super::GitLimits);
    /// [`GitError::Failed`] when a commit is not in the repository; [`GitError::Run`], [`GitError::OutputTooLong`],
    /// [`GitError::NotACommit`], [`GitError::NotANumber`] and [`GitError::Unexpected`].
    pub async fn range_log(&self, base: &CommitId, head: &CommitId) -> Result<Vec<CommitRecord>, GitError> {
        let operation = GitOperation::Log;
        let range = format!("{base}..{head}");
        let one_more = self.limits.max_commits.saturating_add(1).to_string();
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("log"),
                    arg("-z"),
                    arg("--topo-order"),
                    arg("--reverse"),
                    arg("--max-count"),
                    arg(&one_more),
                    arg("--format=%H%x00%at%x00%s"),
                    arg(&range),
                    arg("--"),
                ],
            )
            .await?;
        let bytes = whole(operation, &captured.stdout, self.limits)?;
        let commits = parse_log(operation, bytes)?;
        if commits.len() > self.limits.max_commits {
            return Err(GitError::TooManyCommits {
                limit: self.limits.max_commits,
            });
        }
        Ok(commits)
    }

    /// The files that differ between the commit this worktree is at and the files in it, the new ones included, with the
    /// lines each differs by. The repository is not changed: the files are read through an index and an object directory
    /// of a scratch directory that is removed again.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the directory is no repository; [`GitError::Run`], [`GitError::OutputTooLong`],
    /// [`GitError::Unexpected`], [`GitError::NotUtf8`], [`GitError::NotARepoPath`] and [`GitError::Scratch`].
    pub async fn uncommitted_changes(&self) -> Result<Vec<FileChange>, GitError> {
        let objects = self.objects_dir().await?;
        let scratch = tempfile::tempdir().map_err(|source| GitError::Scratch {
            source: ScratchError::Make(source),
        })?;
        let read = self.scratch_handle(scratch.path(), &objects)?.read_tree_changes().await;
        if let Err(source) = scratch.close() {
            // The scratch directory is in the temporary directory of the machine and holds nothing that matters: a
            // failure to remove it must not lose what was read.
            tracing::warn!(error = %source, "cannot remove the scratch directory of a read of the uncommitted changes");
        }
        read
    }

    /// What a run changed whose worktree was at `before` when it started: the commits and the files between `before` and
    /// the head now, and what is uncommitted. Without `before` nothing is known between the heads.
    ///
    /// # Errors
    /// The errors of [`Git::range_log`], [`Git::range_changes`] and [`Git::uncommitted_changes`].
    pub async fn work_changes(&self, before: Option<&CommitId>) -> Result<WorkChanges, GitError> {
        let head_after = self.rev_parse(&Revision::Head).await?;
        let (commits, files) = match before {
            Some(before) if *before != head_after => (
                self.range_log(before, &head_after).await?,
                self.range_changes(before, &head_after).await?,
            ),
            Some(_) | None => (Vec::new(), Vec::new()),
        };
        let uncommitted = self.uncommitted_changes().await?;
        Ok(WorkChanges {
            head_after,
            commits,
            files,
            uncommitted,
        })
    }

    /// The directory the repository keeps its objects in, shared by all its worktrees.
    async fn objects_dir(&self) -> Result<PathBuf, GitError> {
        let operation = GitOperation::GitPath;
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[arg("rev-parse"), arg("--git-path"), arg("objects")],
            )
            .await?;
        let printed = text(operation, &captured.stdout)?;
        // In the main checkout git prints the path relative to the directory it ran in; `join` keeps an absolute one.
        Ok(self.dir.as_path().join(printed.trim_end()))
    }

    /// This handle with an index and an object directory in `scratch`, and the repository's own objects readable
    /// through it: what it adds to the index and writes as objects stays in `scratch`.
    fn scratch_handle(&self, scratch: &Path, objects: &Path) -> Result<Self, GitError> {
        let name = |name: &'static str| {
            EnvName::from_str(name).map_err(|source| GitError::Scratch {
                source: ScratchError::Name { name, source },
            })
        };
        let text = |path: &Path| {
            path.to_str().map(str::to_owned).ok_or_else(|| GitError::Scratch {
                source: ScratchError::NotText(path.to_path_buf()),
            })
        };
        let new_objects = scratch.join("objects");
        std::fs::create_dir(&new_objects).map_err(|source| GitError::Scratch {
            source: ScratchError::Make(source),
        })?;
        Ok(self
            .clone()
            .with_env(name("GIT_INDEX_FILE")?, text(&scratch.join("index"))?)
            .with_env(name("GIT_OBJECT_DIRECTORY")?, text(&new_objects)?)
            .with_env(name("GIT_ALTERNATE_OBJECT_DIRECTORIES")?, text(objects)?))
    }

    /// With an index of its own that nothing else sees: the head's tree read in, every file of the worktree added, and
    /// the difference between that and the head.
    async fn read_tree_changes(&self) -> Result<Vec<FileChange>, GitError> {
        self.run_ok(GitOperation::ReadTree, Reach::Local, &[arg("read-tree"), arg("HEAD")])
            .await?;
        self.run_ok(GitOperation::Add, Reach::Local, &[arg("add"), arg("-A"), arg("--")])
            .await?;
        let operation = GitOperation::Diff;
        let mut listed = Vec::new();
        for format in ["--name-status", "--numstat"] {
            let args = [
                arg("diff"),
                arg(format),
                arg("-z"),
                arg("-M"),
                arg("--no-ext-diff"),
                arg("--no-textconv"),
                arg("--cached"),
                arg("HEAD"),
                arg("--"),
            ];
            let captured = self.run_ok(operation, Reach::Local, &args).await?;
            listed.push(whole(operation, &captured.stdout, self.limits)?.to_vec());
        }
        match listed.as_slice() {
            [names, counts] => join_changes(
                operation,
                parse_name_status(operation, names)?,
                parse_numstat(operation, counts)?,
            ),
            _ => Err(GitError::Unexpected {
                operation,
                output: String::new(),
            }),
        }
    }
}

/// Reads the output of `git log -z --format=%H%x00%at%x00%s`: three NUL-separated fields for each commit.
fn parse_log(operation: GitOperation, bytes: &[u8]) -> Result<Vec<CommitRecord>, GitError> {
    // `-z` ends every commit, the last too, with a NUL.
    let Some(bytes) = bytes.strip_suffix(&[0]) else {
        return if bytes.is_empty() {
            Ok(Vec::new())
        } else {
            Err(unexpected_log(operation, bytes))
        };
    };
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let unexpected = || unexpected_log(operation, bytes);
    let (chunks, rest) = fields.as_chunks::<LOG_FIELDS>();
    if !rest.is_empty() {
        return Err(unexpected());
    }
    let mut commits = Vec::with_capacity(chunks.len());
    for [id, seconds, subject] in chunks {
        let id = std::str::from_utf8(id)
            .map_err(|_unused| unexpected())
            .and_then(|text| {
                text.parse()
                    .map_err(|source| GitError::NotACommit { operation, source })
            })?;
        let seconds_text = std::str::from_utf8(seconds).map_err(|_unused| unexpected())?;
        let seconds: i64 = seconds_text.parse().map_err(|source| GitError::NotANumber {
            operation,
            text: seconds_text.to_owned(),
            source,
        })?;
        let authored_at = time::UtcDateTime::from_unix_timestamp(seconds)
            .ok()
            .and_then(|utc| Timestamp::from_utc(utc).ok())
            .ok_or_else(unexpected)?;
        commits.push(CommitRecord {
            id,
            // A subject in another encoding has its bad bytes replaced: it is shown to a human, never matched.
            subject: String::from_utf8_lossy(subject).into_owned(),
            authored_at,
        });
    }
    Ok(commits)
}

/// The error for a log that is not what git prints for the format.
fn unexpected_log(operation: GitOperation, bytes: &[u8]) -> GitError {
    GitError::Unexpected {
        operation,
        output: String::from_utf8_lossy(bytes).chars().take(200).collect(),
    }
}
